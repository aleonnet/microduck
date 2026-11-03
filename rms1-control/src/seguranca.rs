//! A única dona do corpo.
//!
//! Ninguém acima dela tem um [`Corpo`], então ninguém acima dela consegue mandar um quadro: quem garante é
//! o compilador, e não uma regra que alguém precisa lembrar. Regras incondicionais:
//!
//! - **Valor não finito é recusado**, nunca limitado: limitar `NaN` daria um valor plausível na ponta.
//! - **O resto é limitado à faixa, e reportado**, porque um cliente precisa saber por que o robô não obedeceu.
//! - **Desarmada, não envia nada que mova.** Ligar o processo não move o robô. Só a telemetria sai desarmada: registrar,
//!   cancelar, assinar e responder, e nada mais cabe em [`Pedido`].
//! - **Sem comando dentro do prazo, as rodas zeram.** A torre fica: uma torre parada é inofensiva, e uma
//!   velocidade velha leva o robô à parede.

use std::time::{Duration, Instant};

use rms1_proto::barramento::Pedido;
use rms1_proto::comando::{
    EIXO_PITCH, EIXO_YAW, TAREFA_PRIMEIRA, TAREFA_ULTIMA, TAXA_MOVENDO, TAXA_PARADA, byte_do_no,
};
use rms1_proto::{Comando, Nos, PermissaoForma, SeqPorComando};
use rms1d_params::{Params, Permissao};

use crate::cinematica::rpm_de_twist;
use crate::corpo::Corpo;
use crate::intencoes::{Leitura, Modo};
use crate::saude::Saude;

/// Nomes para o fio, escritos por extenso para renomear um tipo não quebrar um cliente.
pub const LIMITE_PRAZO: &str = "deadman";
pub const LIMITE_VELOCIDADE: &str = "max_velocity";
pub const LIMITE_RPM: &str = "max_rpm";
pub const LIMITE_TORRE: &str = "gimbal_range";
pub const LIMITE_NAO_FINITO: &str = "not_finite";
pub const LIMITE_DESARMADO: &str = "disarmed";

/// `0x3f/0x19`, kit de programação: `rm_define.chassis_sdk_mode`.
const MODO_CHASSI_PROGRAMA: u8 = 1;
/// `_stop_rotate` da DJI (`rm_ctrl.py:1486`).
const TAXA_ZERO: Comando = Comando::TorreTaxa {
    yaw_ddeg_s: 0,
    pitch_ddeg_s: 0,
    controle: TAXA_PARADA,
};

/// O que foi aplicado neste ciclo.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Aplicado {
    /// O que as rodas ou o seguir receberam, depois de todo limite.
    pub twist: [f64; 3],
    /// O último alvo da torre, no contrato, depois do limite. A taxa do chassi seguindo a move sem mudar isto: sem
    /// telemetria, não há como dizer onde ela está.
    pub torre: Option<[f64; 2]>,
    pub limitado_por: Vec<&'static str>,
}

pub struct Seguranca<C: Corpo> {
    corpo: C,
    nos: Nos,
    seq: SeqPorComando,
    permissao: PermissaoForma,
    prazo: Duration,
    vel_max: f64,
    giro_max: f64,
    rpm_max: f64,
    raio: f64,
    meia_soma: f64,
    yaw_max: f64,
    pitch_faixa: (f64, f64),
    giro_torre_max: f64,
    coordenada: u8,
    velocidade_torre: u16,
    reenvio: Duration,
    vezes_modo_ao_armar: u32,
    vezes_zero_ao_desarmar: u32,
    seq_barramento: u16,
    armado: bool,
    modo: Modo,
    ultimo_alvo: Option<((i16, i16), Instant)>,
    torre_aplicada: Option<[f64; 2]>,
    taxa_ligada: bool,
    tarefa: u8,
}

impl<C: Corpo> Seguranca<C> {
    pub fn new(corpo: C, p: &Params) -> Self {
        Self {
            corpo,
            nos: Nos {
                remetente: byte_do_no(p.nos.remetente),
                remetente_alavanca: byte_do_no(p.nos.remetente_alavanca),
                chassi: byte_do_no(p.nos.chassi),
                torre: byte_do_no(p.nos.torre),
            },
            seq: SeqPorComando::default(),
            permissao: match p.corpo.permissao {
                Permissao::Dji => PermissaoForma::Dji,
                Permissao::Proroklab => PermissaoForma::Proroklab,
            },
            prazo: Duration::from_millis(p.seguranca.prazo_ms),
            vel_max: p.seguranca.velocidade_max_m_s,
            giro_max: p.seguranca.giro_max_grau_s.to_radians(),
            rpm_max: p.seguranca.rpm_max,
            raio: p.geometria.raio_roda_m,
            meia_soma: p.geometria.meia_soma_m,
            yaw_max: p.seguranca.yaw_max_grau,
            pitch_faixa: (p.seguranca.pitch_min_grau, p.seguranca.pitch_max_grau),
            giro_torre_max: p.seguranca.giro_torre_max_grau_s,
            coordenada: p.torre.coordenada,
            velocidade_torre: p.torre.velocidade,
            reenvio: Duration::from_millis(p.torre.reenvio_ms),
            vezes_modo_ao_armar: p.seguranca.vezes_modo_ao_armar,
            vezes_zero_ao_desarmar: p.seguranca.vezes_zero_ao_desarmar,
            seq_barramento: 0,
            armado: false,
            modo: Modo::Livre,
            ultimo_alvo: None,
            torre_aplicada: None,
            taxa_ligada: false,
            tarefa: 0,
        }
    }

    pub fn armado(&self) -> bool {
        self.armado
    }

    pub fn ler_corpo(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.corpo.ler(buf)
    }

    fn enviar(&mut self, c: Comando, saude: &Saude) {
        let seq = self.seq.proxima(c.id());
        let bytes = match c.quadro(&self.nos, seq).montar() {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(erro = %e, "quadro não montado");
                return;
            }
        };
        use std::sync::atomic::Ordering::Relaxed;
        match self.corpo.enviar(&bytes) {
            Ok(()) => saude.erros_seguidos.store(0, Relaxed),
            Err(e) => {
                let n = saude.erros_seguidos.fetch_add(1, Relaxed) + 1;
                if n == 1 || n.is_multiple_of(100) {
                    tracing::warn!(erro = %e, seguidos = n, "envio ao corpo falhou");
                }
            }
        }
    }

    /// A telemetria: o único caminho desses quadros. Erro aqui tem contador próprio, porque o corpo simulado sem página não
    /// recebe, e isso não é o corpo recusando comando.
    pub fn enviar_barramento(&mut self, p: Pedido, saude: &Saude) {
        let seq = self.seq_barramento;
        self.seq_barramento = seq.wrapping_add(1);
        let bytes = match p.quadro(seq).montar() {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(erro = %e, "pedido não montado");
                return;
            }
        };
        if let Err(e) = self.corpo.enviar(&bytes) {
            let n = saude
                .erros_barramento
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                + 1;
            if n == 1 || n.is_multiple_of(100) {
                tracing::debug!(erro = %e, seguidos = n, "pedido de telemetria não chegou ao corpo");
            }
        }
    }

    /// Ligar ao corpo: o modo do chassi, e o de quem segue quem. A permissão começa no próximo ciclo.
    pub fn armar(&mut self, saude: &Saude) {
        if self.armado {
            return;
        }
        for _ in 0..self.vezes_modo_ao_armar {
            self.enviar(Comando::ModoChassi(MODO_CHASSI_PROGRAMA), saude);
        }
        self.armado = true;
        self.configurar_modo(saude);
        self.ultimo_alvo = None;
        tracing::info!(modo = self.modo.nome(), "armado");
    }

    pub fn desarmar(&mut self, saude: &Saude) {
        if !self.armado {
            return;
        }
        // Parar como a DJI, conforme o modo (`_set_chassis_stop_depend_mode`): rodas não valem seguindo a torre.
        let parar = match self.modo {
            Modo::ChassiSegueTorre => Comando::Seguir {
                vx: 0.0,
                vy: 0.0,
                angulo: 0.0,
            },
            Modo::Livre | Modo::TorreSegueChassi => Comando::Rodas([0; 4]),
        };
        for _ in 0..self.vezes_zero_ao_desarmar {
            self.enviar(parar, saude);
        }
        if self.taxa_ligada {
            self.enviar(TAXA_ZERO, saude);
            self.taxa_ligada = false;
        }
        self.armado = false;
        tracing::info!("desarmado");
    }

    /// Os comandos que a DJI usa em `RobotCtrl.set_mode_attr` (`rm_ctrl.py`): `0x3f/0x46` vai ao processo
    /// `dji_hdvt_uav`, que paramos; o resto é chassi e torre.
    fn configurar_modo(&mut self, saude: &Saude) {
        let torre = match self.modo {
            Modo::Livre | Modo::ChassiSegueTorre => 0,
            Modo::TorreSegueChassi => 2,
        };
        self.enviar(Comando::ModoTorre(torre), saude);
        self.enviar(Comando::Sobrepor(0), saude);
        self.ultimo_alvo = None;
    }

    fn limitar_twist(&self, leitura: &Leitura, limites: &mut Vec<&'static str>) -> [f64; 3] {
        let pedido = leitura.twist;
        if pedido.iter().any(|v| !v.is_finite()) {
            limites.push(LIMITE_NAO_FINITO);
            return [0.0; 3];
        }
        let vencido = leitura.idade_twist.is_none_or(|i| i > self.prazo);
        if vencido {
            if pedido != [0.0; 3] {
                limites.push(LIMITE_PRAZO);
            }
            return [0.0; 3];
        }
        let lim = |v: f64, m: f64| v.clamp(-m, m);
        let saida = [
            lim(pedido[0], self.vel_max),
            lim(pedido[1], self.vel_max),
            lim(pedido[2], self.giro_max),
        ];
        if saida != pedido {
            limites.push(LIMITE_VELOCIDADE);
        }
        saida
    }

    /// Rodas dentro do limite, encolhendo as quatro juntas: cortar só a que passou mudaria o rumo. Devolve
    /// também quanto encolheu, porque o movimento aplicado encolhe junto.
    fn rodas(&self, twist: [f64; 3], limites: &mut Vec<&'static str>) -> ([i16; 4], f64) {
        let rpm = rpm_de_twist(twist, self.raio, self.meia_soma);
        let maior = rpm.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
        let escala = if maior > self.rpm_max {
            limites.push(LIMITE_RPM);
            self.rpm_max / maior
        } else {
            1.0
        };
        (rpm.map(|v| (v * escala).round() as i16), escala)
    }

    /// Alvo do contrato para graus da DJI: os dois sinais são invertidos (yaw positivo à esquerda e pitch
    /// positivo para baixo no contrato; à direita e para cima na DJI). `true` se foi limitado. Com a torre
    /// seguindo o chassi, o yaw não é nosso: nem vai ao corpo, nem conta como limitado.
    fn alvo_torre(&self, yaw: f64, pitch: f64) -> Option<((i16, i16), bool)> {
        if !yaw.is_finite() || !pitch.is_finite() {
            return None;
        }
        let pedido_y = if self.modo == Modo::TorreSegueChassi {
            0.0
        } else {
            -yaw.to_degrees()
        };
        let pedido_p = -pitch.to_degrees();
        let y = pedido_y.clamp(-self.yaw_max, self.yaw_max);
        let p = pedido_p.clamp(self.pitch_faixa.0, self.pitch_faixa.1);
        Some((
            ((y * 10.0).round() as i16, (p * 10.0).round() as i16),
            y != pedido_y || p != pedido_p,
        ))
    }

    fn proxima_tarefa(&mut self) -> u8 {
        self.tarefa = if (TAREFA_PRIMEIRA..TAREFA_ULTIMA).contains(&self.tarefa) {
            self.tarefa + 1
        } else {
            TAREFA_PRIMEIRA
        };
        self.tarefa
    }

    /// O ciclo: o único caminho até o corpo.
    pub fn aplicar(&mut self, leitura: &Leitura, agora: Instant, saude: &Saude) -> Aplicado {
        let mudou_modo = leitura.modo != self.modo;
        self.modo = leitura.modo;
        if !self.armado {
            let vivo = leitura.idade_twist.is_some_and(|i| i <= self.prazo);
            let limitado_por = if vivo && leitura.twist != [0.0; 3] {
                vec![LIMITE_DESARMADO]
            } else {
                Vec::new()
            };
            return Aplicado {
                twist: [0.0; 3],
                torre: self.torre_aplicada,
                limitado_por,
            };
        }
        if mudou_modo {
            self.configurar_modo(saude);
        }
        let mut limites = Vec::new();
        self.enviar(Comando::Permissao(self.permissao), saude);
        let mut twist = self.limitar_twist(leitura, &mut limites);
        let mut giro_para_torre = 0.0;
        match self.modo {
            Modo::Livre | Modo::TorreSegueChassi => {
                let (rodas, escala) = self.rodas(twist, &mut limites);
                self.enviar(Comando::Rodas(rodas), saude);
                twist = twist.map(|v| v * escala);
            }
            Modo::ChassiSegueTorre => {
                // O contrato tem y à esquerda; a DJI, à direita (`_move_ctrl` em `rm_ctrl.py`). Subtração, porque
                // `-0.0` sairia no fio com o bit de sinal, e parado tem de ser zero.
                self.enviar(
                    Comando::Seguir {
                        vx: twist[0] as f32,
                        vy: (0.0 - twist[1]) as f32,
                        angulo: 0.0,
                    },
                    saude,
                );
                // O giro vira taxa da torre: o aplicado é o que cabe nela.
                let limite = self.giro_torre_max.to_radians();
                giro_para_torre = twist[2].clamp(-limite, limite);
                if giro_para_torre != twist[2] {
                    limites.push(LIMITE_VELOCIDADE);
                    twist[2] = giro_para_torre;
                }
            }
        }
        self.torre(leitura, giro_para_torre, agora, saude, &mut limites);
        Aplicado {
            twist,
            torre: self.torre_aplicada,
            limitado_por: limites,
        }
    }

    /// No chassi segue a torre, o giro pedido move a torre por taxa, como a alavanca da DJI, e o chassi vai
    /// atrás. Fora disso, ângulo: reenviado enquanto não muda, salvo seguindo a torre, porque um alvo em
    /// relação ao chassi reenviado faria o chassi nunca parar.
    fn torre(
        &mut self,
        leitura: &Leitura,
        giro: f64,
        agora: Instant,
        saude: &Saude,
        limites: &mut Vec<&'static str>,
    ) {
        // A velocidade pedida vale até o prazo, como o giro das rodas: sem cliente, a torre para.
        let taxa = if leitura.idade_taxa.is_some_and(|i| i <= self.prazo) {
            leitura.taxa_torre
        } else {
            [0.0; 2]
        };
        let taxa = if taxa.iter().all(|v| v.is_finite()) {
            taxa
        } else {
            limites.push(LIMITE_NAO_FINITO);
            [0.0; 2]
        };
        let alvo = match leitura.torre {
            None => None,
            Some([yaw, pitch]) => match self.alvo_torre(yaw, pitch) {
                None => {
                    limites.push(LIMITE_NAO_FINITO);
                    None
                }
                Some((a, limitado)) => {
                    if limitado {
                        limites.push(LIMITE_TORRE);
                    }
                    Some(a)
                }
            },
        };
        // Com a torre seguindo o chassi, o yaw não é nosso.
        let pedido_yaw = if self.modo == Modo::TorreSegueChassi {
            0.0
        } else {
            taxa[0]
        };
        let yaw = pedido_yaw - giro.to_degrees();
        if yaw != 0.0 || taxa[1] != 0.0 {
            let lim = self.giro_torre_max;
            if yaw.abs() > lim || taxa[1].abs() > lim {
                limites.push(LIMITE_VELOCIDADE);
            }
            // A cada ciclo, como `rotate_with_speed` manda `0x04/0x0c`, e sem tarefa nova: o controlador não recomeça a rampa.
            self.enviar(
                Comando::TorreTaxa {
                    yaw_ddeg_s: (yaw.clamp(-lim, lim) * 10.0).round() as i16,
                    pitch_ddeg_s: (taxa[1].clamp(-lim, lim) * 10.0).round() as i16,
                    controle: TAXA_MOVENDO,
                },
                saude,
            );
            self.taxa_ligada = true;
            return;
        }
        if self.taxa_ligada {
            self.enviar(TAXA_ZERO, saude);
            self.taxa_ligada = false;
        }
        let Some(alvo) = alvo else {
            return;
        };
        self.torre_aplicada = Some([
            -(f64::from(alvo.0) / 10.0).to_radians(),
            -(f64::from(alvo.1) / 10.0).to_radians(),
        ]);
        let reenviar = self.modo != Modo::ChassiSegueTorre;
        let devido = match self.ultimo_alvo {
            None => true,
            Some((a, em)) => a != alvo || (reenviar && agora.duration_since(em) >= self.reenvio),
        };
        if !devido {
            return;
        }
        let eixos = match self.modo {
            Modo::TorreSegueChassi => EIXO_PITCH,
            Modo::Livre | Modo::ChassiSegueTorre => EIXO_YAW | EIXO_PITCH,
        };
        let tarefa = self.proxima_tarefa();
        self.enviar(
            Comando::TorreAngulos {
                tarefa,
                yaw_ddeg: alvo.0,
                pitch_ddeg: alvo.1,
                eixos,
                coordenada: self.coordenada,
                velocidade: self.velocidade_torre,
            },
            saude,
        );
        self.ultimo_alvo = Some((alvo, agora));
    }
}
