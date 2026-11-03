//! O que o `rms1d` calcula por conta própria, ao lado do controlador da DJI, em modo sombra: nada aqui muda o que se
//! envia ao robô nem o `robot.state`.
//!
//! **Por quê** (medido no robô em 2026-11-03, nos dois modos, `out/2026-11-03-firmware-controlador/` no laboratório): o
//! controlador integra a aceleração para dar velocidade e posição, e erra quando o robô inclina, até 13 m/s parado na mão;
//! o bit de levantado dele nunca acendeu. Atitude, aceleração, giro e rodas acompanham o movimento real. O pato faz o
//! mesmo: odometria e levantado próprios (`odometry/`, `duck-control/src/pickup.rs`).
//!
//! - **Velocidade:** das quatro rodas, pela inversa da cinemática, na média dos últimos `janela_ms` do relógio do robô,
//!   e não de um número de pacotes, que num buraco da gravação arrastaria amostra velha; zero quando o controlador
//!   marca parado, porque parado as rodas só dão ruído (até 0,08 m/s, medido).
//! - **Posição:** essa velocidade girada pelo rumo do controlador e integrada pelo relógio do pacote, a partir de onde o
//!   controlador estava no primeiro pacote.
//! - **Rumo:** o do controlador, que é o giro integrado por ele. O nosso, pelo giro da IMU, sai ao lado para comparar.
//! - **Levantado:** quando o controlador acusa mais velocidade do que as rodas, por tempo: é o erro dele ao inclinar, e
//!   no ar as rodas não andam. Mecanismo do `s1d` (`s1d/src/tick.rs`, `LiftDetector`), conferido nas gravações.
//! - **Gravidade:** pela atitude do controlador. Com o robô quieto, ela é conferida contra a aceleração: a inclinação
//!   pela atitude e a pela aceleração, em módulo, que não depende do sinal da inclinação, ainda sem prova.
//!
//! **Sem prova:** o levantado com o robô carregado e as rodas girando, ou levantado devagar demais para o controlador
//! acusar velocidade; o escorregamento das rodas no chão. A prova é o robô em modo sombra.

use std::collections::VecDeque;

use rms1_proto::Quadro;
use rms1_proto::barramento::{Layout, Pacote, id};
use rms1d_params::{Estimativa as Limiares, Geometria, Params};

use crate::cinematica::twist_de_rpm;
use crate::telemetria::{BIT_PARADO, Medidas};

/// Um passo do cálculo, nos eixos do contrato: x à frente, y à esquerda, z para cima.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Calculo {
    /// m/s, no corpo, pelas rodas.
    pub velocidade_corpo: [f64; 2],
    /// rad/s, positivo à esquerda, pelas rodas: só para comparar com a IMU, que é melhor (medido: 5% a 33% acima).
    pub giro_rodas: f64,
    /// m, no mundo do controlador.
    pub posicao: [f64; 2],
    /// Graus, positivo à esquerda: o do controlador.
    pub rumo: f64,
    /// Graus: o giro da IMU integrado por nós, a partir do rumo do controlador no primeiro pacote.
    pub rumo_pelo_giro: f64,
    pub levantado: bool,
    /// m/s: a velocidade do controlador no plano menos a nossa.
    pub discordancia: f64,
    /// Em g, no tronco, pela atitude do controlador: `[0, 0, -1]` em pé, como o contrato pede.
    pub gravidade: [f64; 3],
    /// Graus, com o robô quieto: a inclinação pela atitude menos a pela aceleração, em módulo. `None` fora do quieto.
    pub divergencia: Option<f64>,
    /// A divergência passou de `divergencia_grau`.
    pub divergente: bool,
}

/// O estado do cálculo, um passo por pacote.
pub struct Estimativa {
    l: Limiares,
    g: Geometria,
    /// `(relógio em ms, giro do corpo pelas rodas)`.
    janela: VecDeque<(u64, [f64; 3])>,
    ultimo_ms: Option<u32>,
    /// ms desde o primeiro pacote, somando as diferenças do relógio do robô.
    relogio: u64,
    posicao: [f64; 2],
    rumo_giro: f64,
    acima_desde: Option<u64>,
    abaixo_desde: Option<u64>,
    levantado: bool,
}

/// A inclinação, em graus, de um vetor que aponta para cima no tronco quando o robô está em pé.
fn inclinacao_grau(v: [f64; 3]) -> Option<f64> {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (n > 0.0).then(|| (v[2] / n).clamp(-1.0, 1.0).acos().to_degrees())
}

/// A gravidade no tronco pelo quaternion tronco para mundo `[w, x, y, z]`: o "para baixo" do mundo visto do tronco.
fn gravidade_do_quaternion(q: [f64; 4]) -> [f64; 3] {
    let [w, x, y, z] = q;
    [
        -2.0 * (x * z - w * y),
        -2.0 * (y * z + w * x),
        -(1.0 - 2.0 * (x * x + y * y)),
    ]
}

impl Estimativa {
    pub fn new(l: &Limiares, g: &Geometria) -> Self {
        Self {
            l: *l,
            g: *g,
            janela: VecDeque::new(),
            ultimo_ms: None,
            relogio: 0,
            posicao: [0.0; 2],
            rumo_giro: 0.0,
            acima_desde: None,
            abaixo_desde: None,
            levantado: false,
        }
    }

    /// Um pacote, nas unidades do contrato, com os bits de estado do chassi.
    pub fn passo(&mut self, m: &Medidas, estado: u32) -> Calculo {
        let dt_ms = match self.ultimo_ms {
            None => {
                self.posicao = [m.posicao[0], m.posicao[1]];
                self.rumo_giro = m.rumo;
                0
            }
            Some(u) => {
                let d = u64::from(m.relogio_ms.wrapping_sub(u));
                if d > self.l.lacuna_ms {
                    // Depois da lacuna, nada do que veio antes vale para a média.
                    self.janela.clear();
                    0
                } else {
                    d
                }
            }
        };
        self.ultimo_ms = Some(m.relogio_ms);
        self.relogio += dt_ms;
        let dt = dt_ms as f64 / 1000.0;

        let t = twist_de_rpm(m.rodas_rpm, self.g.raio_roda_m, self.g.meia_soma_m);
        self.janela.push_back((self.relogio, t));
        while self
            .janela
            .front()
            .is_some_and(|(r, _)| self.relogio - r >= self.l.janela_ms)
        {
            self.janela.pop_front();
        }
        let n = self.janela.len() as f64;
        let media = |i: usize| self.janela.iter().map(|(_, v)| v[i]).sum::<f64>() / n;
        let parado = estado & BIT_PARADO != 0;
        let (v, giro_rodas) = if parado {
            ([0.0; 2], 0.0)
        } else {
            ([media(0), media(1)], media(2))
        };

        let (s, c) = m.rumo.to_radians().sin_cos();
        self.posicao[0] += (v[0] * c - v[1] * s) * dt;
        self.posicao[1] += (v[0] * s + v[1] * c) * dt;
        self.rumo_giro += m.giro[2].to_degrees() * dt;

        let discordancia = m.velocidade_corpo[0].hypot(m.velocidade_corpo[1]) - v[0].hypot(v[1]);
        self.levantado = self.levantar(discordancia);

        let gravidade = gravidade_do_quaternion(m.quaternion());
        let a = m.aceleracao;
        let modulo = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        // Giro baixo e módulo de 1 g não bastam: freando ou arrancando, a aceleração inclina sem mudar de módulo (medido
        // em 2026-11-03: 11,5° andando, a 121 s da gravação do modo DJI). Quieto é também parado, pelo controlador.
        let quieto = parado
            && m.giro.iter().all(|w| w.abs() < self.l.quieto_giro_rad_s)
            && (modulo - 1.0).abs() < self.l.quieto_aceleracao_g;
        let divergencia = if quieto {
            match (inclinacao_grau(gravidade.map(|x| -x)), inclinacao_grau(a)) {
                (Some(pela_atitude), Some(pela_aceleracao)) => {
                    Some((pela_atitude - pela_aceleracao).abs())
                }
                _ => None,
            }
        } else {
            None
        };

        Calculo {
            velocidade_corpo: v,
            giro_rodas,
            posicao: self.posicao,
            rumo: m.rumo,
            rumo_pelo_giro: self.rumo_giro,
            levantado: self.levantado,
            discordancia,
            gravidade,
            divergencia,
            divergente: divergencia.is_some_and(|d| d > self.l.divergencia_grau),
        }
    }

    /// Acima do limiar por `levantado_tempo_ms` marca; abaixo do de soltar por `soltar_tempo_ms` solta; entre os dois,
    /// fica como está.
    fn levantar(&mut self, d: f64) -> bool {
        let agora = self.relogio;
        if d > self.l.levantado_limiar_m_s {
            self.abaixo_desde = None;
            let desde = *self.acima_desde.get_or_insert(agora);
            if agora - desde >= self.l.levantado_tempo_ms {
                return true;
            }
        } else if d < self.l.soltar_limiar_m_s {
            self.acima_desde = None;
            let desde = *self.abaixo_desde.get_or_insert(agora);
            if agora - desde >= self.l.soltar_tempo_ms {
                return false;
            }
        } else {
            self.acima_desde = None;
            self.abaixo_desde = None;
        }
        self.levantado
    }
}

/// Um pacote reprocessado: o que o controlador mandou e o que nós calculamos com ele.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Linha {
    /// Segundos desde o primeiro pacote, pelo relógio do robô.
    pub t: f64,
    pub medidas: Medidas,
    pub estado: u32,
    pub calculo: Calculo,
}

/// Uma gravação: quantos quadros ela tem, e os pacotes da nossa assinatura, reprocessados.
pub struct Reprocessada {
    pub quadros: usize,
    pub linhas: Vec<Linha>,
}

/// O `strace` das leituras de um processo (`rms1_proto::strace`), na ordem da nossa assinatura.
pub fn de_strace(texto: &[u8], p: &Params) -> Reprocessada {
    let qs: Vec<Quadro> = rms1_proto::strace::quadros(texto)
        .into_iter()
        .map(|(_, q)| q)
        .collect();
    Reprocessada {
        quadros: qs.len(),
        linhas: reprocessar(&qs, &Layout::da_assinatura(), p),
    }
}

/// Os bytes de `corpo.gravar_recebido` ([`crate::gravacao::ler_recebido`]), na ordem da nossa assinatura.
pub fn de_recebido(bytes: &[u8], p: &Params) -> Reprocessada {
    let qs = rms1_proto::Leitor::new().empurrar(bytes);
    Reprocessada {
        quadros: qs.len(),
        linhas: reprocessar(&qs, &Layout::da_assinatura(), p),
    }
}

/// Uma gravação inteira, pacote a pacote: os pacotes de telemetria do tamanho que `layout` diz, na ordem em que chegaram.
pub fn reprocessar<'a>(
    quadros: impl IntoIterator<Item = &'a Quadro>,
    layout: &Layout,
    p: &Params,
) -> Vec<Linha> {
    let mut e = Estimativa::new(&p.estimativa, &p.geometria);
    let mut saida = Vec::new();
    let mut t_ms: u64 = 0;
    let mut ultimo: Option<u32> = None;
    for q in quadros {
        if q.resposta || (q.cmd_set, q.cmd_id) != id::PACOTE || q.dados.len() != layout.total {
            continue;
        }
        let Ok(pac) = Pacote::ler(&q.dados, layout) else {
            continue;
        };
        if let Some(u) = ultimo {
            t_ms += u64::from(pac.tempo_ms.wrapping_sub(u));
        }
        ultimo = Some(pac.tempo_ms);
        let m = Medidas::de(&pac);
        saida.push(Linha {
            t: t_ms as f64 / 1000.0,
            medidas: m,
            estado: pac.estado,
            calculo: e.passo(&m, pac.estado),
        });
    }
    saida
}
