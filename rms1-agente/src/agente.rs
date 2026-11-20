//! O núcleo do agente, sem rede: entram o pedido validado, o estado do corpo e as detecções; saem o que pedir ao `rms1d`
//! e como cada habilidade terminou.
//!
//! As regras:
//!
//! - **Uma habilidade por vez.** Um pedido novo encerra a anterior como interrompida, e começa.
//! - **Parar nunca é barrado:** sai mesmo sem o corpo responder ou com o robô desarmado, e encerra a habilidade em curso.
//! - **`robot.stop` é só da ferramenta de parar.** No `rms1d` a parada apaga os pedidos de todas as fontes, inclusive o de
//!   quem pilota. Para arrumar a casa no fim de uma habilidade, o agente solta só o canal dele: quem andava manda as rodas
//!   a zero, e quem girava a torre manda a torre parar.
//! - **O corpo tem dono por canal** (a tabela de fontes do `rms1d`). Com o canal da habilidade na mão de outra fonte por
//!   mais de `perdeu_apos_ms`, ela termina interrompida, dizendo quem assumiu, e não retoma sozinha. Ao desistir, o agente
//!   zera a linha dele no `rms1d`, como em todo fim de habilidade: o pedido de uma fonte superada fica guardado lá, e o
//!   último que ele renovou passaria a valer quando a vez da outra vencesse, com o pedido já dado por interrompido. O zero
//!   na linha do agente não mexe no que a fonte que assumiu pede.
//! - **O agente não arma.** Só age com o robô armado por quem pilota; desarmado no meio, larga a habilidade.
//! - **Toda habilidade termina, e diz como:** cumprida, vencida (o teto da tabela), interrompida ou recusada.
//! - **O pedido contínuo é renovado** dentro do prazo do `rms1d`, que zera o corpo sem pedido novo: se o agente cair, o
//!   robô para sozinho.

use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::tabela::{Habilidade, Tabela};

/// O pedido que a estação validou e mandou cumprir: o número dele lá, a ferramenta e os parâmetros.
#[derive(Debug, Clone)]
pub struct Pedido {
    pub numero: u64,
    pub ferramenta: String,
    pub parametros: Map<String, Value>,
}

/// O que o agente precisa saber do corpo, tirado de `robot.state`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corpo {
    /// Armado por quem pilota (`policy` diferente de `held`).
    pub armado: bool,
    /// Onde o corpo diz que está, em metros (`odom.position`). Sem telemetria, não muda.
    pub posicao: [f64; 2],
}

/// Uma caixa do detector: o rótulo, o centro em fração do quadro (0 a 1, x para a direita e y para baixo) e a nota.
#[derive(Debug, Clone, PartialEq)]
pub struct Caixa {
    pub rotulo: String,
    pub centro: [f64; 2],
    pub nota: f64,
}

/// O nome com que o agente se apresenta ao `rms1d` (`hello`, `client`): a linha dele na tabela de fontes de `rms1d.toml`.
pub const NOME_NO_CORPO: &str = "agente";

/// Como uma habilidade terminou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Como {
    Cumprida,
    Vencida,
    Interrompida,
    Recusada,
}

impl Como {
    pub fn nome(self) -> &'static str {
        match self {
            Self::Cumprida => "cumprida",
            Self::Vencida => "vencida",
            Self::Interrompida => "interrompida",
            Self::Recusada => "recusada",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Desfecho {
    /// O número do pedido na estação.
    pub numero: u64,
    pub ferramenta: String,
    pub como: Como,
    pub motivo: String,
}

/// O que o agente manda fazer.
#[derive(Debug, Clone, PartialEq)]
pub enum Acao {
    /// Uma linha JSON-RPC para o `rms1d`.
    Corpo(Value),
    /// Uma habilidade terminou.
    Terminou(Desfecho),
}

/// A habilidade em curso.
struct Curso {
    numero: u64,
    ferramenta: String,
    comecou: Instant,
    /// Quando o pedido contínuo saiu pela última vez.
    renovado: Instant,
    /// Desde quando o canal da habilidade está na mão de outra fonte; `None` enquanto é do agente, ou de ninguém.
    perdeu_desde: Option<Instant>,
    o_que: OQue,
}

enum OQue {
    Mover {
        /// A velocidade no corpo, em m/s: x à frente, y à esquerda.
        twist: [f64; 2],
        metros: f64,
        /// Onde o corpo estava quando começou.
        de: [f64; 2],
        /// O tempo da distância na velocidade pedida, e o teto.
        por_tempo: Duration,
        teto: Duration,
    },
    Olhar {
        /// O alvo como foi dito, e em palavras.
        dito: String,
        palavras: Vec<String>,
        ganho: f64,
        tolerancia: f64,
        precisa: u32,
        seguidas: u32,
        teto: Duration,
        /// A velocidade da torre em uso, em graus por segundo: yaw à direita, pitch para cima.
        taxa: [f64; 2],
    },
    Procurar {
        dito: String,
        palavras: Vec<String>,
        varredura: f64,
        lado: Duration,
        teto: Duration,
        /// Para que lado a torre vai agora: 1 ou -1.
        sentido: f64,
    },
}

/// Quanto o corpo tem de ter andado, pela posição que ele diz, para a posição valer como medida do caminho.
const ODOMETRIA_ANDOU_M: f64 = 0.02;

pub struct Agente {
    tabela: Tabela,
    corpo: Option<(Corpo, Instant)>,
    curso: Option<Curso>,
    /// O número da próxima requisição ao `rms1d`.
    proximo_id: u64,
}

impl Agente {
    pub fn novo(tabela: Tabela) -> Self {
        Self {
            tabela,
            corpo: None,
            curso: None,
            proximo_id: 1,
        }
    }

    /// A ferramenta da habilidade em curso, se há uma.
    pub fn em_curso(&self) -> Option<&str> {
        self.curso.as_ref().map(|c| c.ferramenta.as_str())
    }

    /// O estado do corpo, a cada `robot.state`.
    pub fn corpo(&mut self, corpo: Corpo, agora: Instant) {
        self.corpo = Some((corpo, agora));
    }

    /// O `rms1d` saiu: o que se sabia do corpo não vale mais.
    pub fn corpo_caiu(&mut self) {
        self.corpo = None;
    }

    fn corpo_recente(&self, agora: Instant) -> Option<Corpo> {
        let vale = Duration::from_millis(self.tabela.geral.corpo_vale_ms);
        self.corpo
            .filter(|(_, quando)| agora.saturating_duration_since(*quando) < vale)
            .map(|(c, _)| c)
    }

    /// `robot.stop`, como requisição: o `rms1d` não aplica a parada mandada como notificação. Só a ferramenta de parar a
    /// usa: no `rms1d` ela apaga os pedidos de todas as fontes.
    fn parar(&mut self) -> Acao {
        let id = self.proximo_id;
        self.proximo_id += 1;
        Acao::Corpo(json!({"jsonrpc": "2.0", "id": id, "method": "robot.stop"}))
    }

    fn andar(twist: [f64; 2]) -> Acao {
        Acao::Corpo(
            json!({"jsonrpc": "2.0", "method": "robot.move", "params": {"vx": twist[0], "vy": twist[1], "vyaw": 0.0}}),
        )
    }

    fn torre(taxa: [f64; 2]) -> Acao {
        let params = if taxa == [0.0; 2] {
            json!({"action": "stop"})
        } else {
            json!({"action": "speed", "yaw": taxa[0], "pitch": taxa[1]})
        };
        Acao::Corpo(json!({"jsonrpc": "2.0", "method": "s1.gimbal", "params": params}))
    }

    fn terminou(numero: u64, ferramenta: &str, como: Como, motivo: impl Into<String>) -> Acao {
        Acao::Terminou(Desfecho {
            numero,
            ferramenta: ferramenta.to_owned(),
            como,
            motivo: motivo.into(),
        })
    }

    /// Solta o canal da habilidade: quem andava manda as rodas a zero, e quem girava a torre manda a torre parar. O
    /// pedido de outra fonte, no mesmo canal ou em outro, segue como está.
    fn soltar(o_que: &OQue) -> Acao {
        match o_que {
            OQue::Mover { .. } => Self::andar([0.0; 2]),
            OQue::Olhar { .. } | OQue::Procurar { .. } => Self::torre([0.0; 2]),
        }
    }

    /// Encerra a habilidade em curso, se há uma: solta o canal dela e diz como terminou.
    fn encerrar(&mut self, como: Como, motivo: &str, acoes: &mut Vec<Acao>) {
        if let Some(c) = self.curso.take() {
            acoes.push(Self::soltar(&c.o_que));
            acoes.push(Self::terminou(c.numero, &c.ferramenta, como, motivo));
        }
    }

    /// Quem tem as rodas e a torre agora, do bloco `control` de `s1.state`: o nome da fonte, ou nada se ninguém tem. `eu`
    /// é a linha da tabela que o `rms1d` diz que esta conexão é: o nome do `hello`, ou a fonte sem nome numa tabela que
    /// não tem a linha do agente. É com ela que o dono é comparado, e não com um nome suposto.
    ///
    /// Com o canal da habilidade em curso na mão de outra fonte por mais de `perdeu_apos_ms`, ela termina interrompida,
    /// dizendo quem assumiu, e o agente solta o canal: o zero na linha dele apaga o último pedido que ele renovou, que
    /// senão valeria quando a vez da outra fonte vencesse. Um instante com outra fonte não conta: o prazo recomeça quando
    /// o canal volta ao agente.
    pub fn donos(
        &mut self,
        eu: &str,
        rodas: Option<&str>,
        torre: Option<&str>,
        agora: Instant,
    ) -> Vec<Acao> {
        let Some(c) = self.curso.as_mut() else {
            return Vec::new();
        };
        let (dono, canal) = match c.o_que {
            OQue::Mover { .. } => (rodas, "as rodas"),
            OQue::Olhar { .. } | OQue::Procurar { .. } => (torre, "a torre"),
        };
        let Some(outra) = dono.filter(|d| *d != eu) else {
            c.perdeu_desde = None;
            return Vec::new();
        };
        let desde = *c.perdeu_desde.get_or_insert(agora);
        if agora.saturating_duration_since(desde)
            < Duration::from_millis(self.tabela.geral.perdeu_apos_ms)
        {
            return Vec::new();
        }
        let c = self.curso.take().expect("conferido acima");
        vec![
            Self::soltar(&c.o_que),
            Self::terminou(
                c.numero,
                &c.ferramenta,
                Como::Interrompida,
                format!("o {outra} assumiu {canal}"),
            ),
        ]
    }

    /// As palavras de um alvo, em minúsculas, sem pontuação.
    fn palavras(dito: &str) -> Vec<String> {
        dito.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// Se a caixa é do alvo: o rótulo é o que uma palavra do alvo quer dizer (a tabela de alvos), ou tem uma palavra dele.
    fn casa(&self, palavras: &[String], rotulo: &str) -> bool {
        let rotulo = rotulo.to_lowercase();
        palavras.iter().any(|p| {
            self.tabela.alvos.get(p).is_some_and(|r| *r == rotulo)
                || rotulo.split_whitespace().any(|r| r == p)
        })
    }

    /// A caixa do alvo de maior nota.
    fn alvo<'a>(&self, palavras: &[String], caixas: &'a [Caixa]) -> Option<&'a Caixa> {
        caixas
            .iter()
            .filter(|c| self.casa(palavras, &c.rotulo))
            .max_by(|a, b| a.nota.total_cmp(&b.nota))
    }

    /// Um pedido validado pela estação.
    pub fn pedido(&mut self, p: Pedido, agora: Instant) -> Vec<Acao> {
        let mut acoes = Vec::new();
        let Some(h) = self
            .tabela
            .habilidades
            .iter()
            .find(|h| h.ferramenta() == p.ferramenta)
            .cloned()
        else {
            let motivo = format!("o agente não tem a habilidade {}", p.ferramenta);
            return vec![Self::terminou(
                p.numero,
                &p.ferramenta,
                Como::Recusada,
                motivo,
            )];
        };
        // A parada nunca é barrada: sai antes de qualquer conferência.
        if let Habilidade::Parar { ferramenta } = &h {
            acoes.push(self.parar());
            if let Some(c) = self.curso.take() {
                acoes.push(Self::terminou(
                    c.numero,
                    &c.ferramenta,
                    Como::Interrompida,
                    "mandaram parar",
                ));
            }
            acoes.push(Self::terminou(
                p.numero,
                ferramenta,
                Como::Cumprida,
                "parado",
            ));
            return acoes;
        }
        let recusa = |motivo: String| {
            vec![Self::terminou(
                p.numero,
                &p.ferramenta,
                Como::Recusada,
                motivo,
            )]
        };
        let Some(corpo) = self.corpo_recente(agora) else {
            return recusa("o corpo não responde: o rms1d não diz o estado".to_owned());
        };
        if !corpo.armado {
            return recusa("o robô não está armado, e o agente não arma".to_owned());
        }
        let texto = |nome: &str| {
            p.parametros
                .get(nome)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .ok_or_else(|| format!("falta o parâmetro {nome}"))
        };
        let o_que = match &h {
            Habilidade::Parar { .. } => unreachable!("a parada saiu acima"),
            Habilidade::Mover {
                direcao,
                metros,
                velocidade,
                folga,
                direcoes,
                ..
            } => {
                let para = match texto(direcao) {
                    Ok(t) => t,
                    Err(e) => return recusa(e),
                };
                let Some([x, y]) = direcoes.get(para) else {
                    let valem: Vec<&str> = direcoes.keys().map(String::as_str).collect();
                    return recusa(format!(
                        "{direcao} desconhecida: {para}; vale {}",
                        valem.join(", ")
                    ));
                };
                let Some(m) = p
                    .parametros
                    .get(metros)
                    .and_then(Value::as_f64)
                    .filter(|m| m.is_finite() && *m > 0.0)
                else {
                    return recusa(format!(
                        "falta o parâmetro {metros}, um número maior que zero"
                    ));
                };
                let norma = x.hypot(*y);
                let por_tempo = Duration::from_secs_f64(m / velocidade);
                OQue::Mover {
                    twist: [x / norma * velocidade, y / norma * velocidade],
                    metros: m,
                    de: corpo.posicao,
                    por_tempo,
                    teto: por_tempo.mul_f64(*folga) + Duration::from_secs(1),
                }
            }
            Habilidade::Olhar {
                alvo,
                ganho,
                tolerancia,
                seguidas,
                teto_s,
                ..
            } => {
                let dito = match texto(alvo) {
                    Ok(t) => t.to_owned(),
                    Err(e) => return recusa(e),
                };
                OQue::Olhar {
                    palavras: Self::palavras(&dito),
                    dito,
                    ganho: *ganho,
                    tolerancia: *tolerancia,
                    precisa: *seguidas,
                    seguidas: 0,
                    teto: Duration::from_secs_f64(*teto_s),
                    taxa: [0.0; 2],
                }
            }
            Habilidade::Procurar {
                alvo,
                varredura,
                lado_s,
                teto_s,
                ..
            } => {
                let dito = match texto(alvo) {
                    Ok(t) => t.to_owned(),
                    Err(e) => return recusa(e),
                };
                OQue::Procurar {
                    palavras: Self::palavras(&dito),
                    dito,
                    varredura: *varredura,
                    lado: Duration::from_secs_f64(*lado_s),
                    teto: Duration::from_secs_f64(*teto_s),
                    sentido: 1.0,
                }
            }
        };
        // O pedido vale: o que estava em curso termina aqui.
        self.encerrar(Como::Interrompida, "chegou outro pedido", &mut acoes);
        match &o_que {
            OQue::Mover { twist, .. } => acoes.push(Self::andar(*twist)),
            OQue::Procurar {
                varredura, sentido, ..
            } => acoes.push(Self::torre([varredura * sentido, 0.0])),
            // Antes de ver o alvo, a torre fica onde está.
            OQue::Olhar { .. } => {}
        }
        self.curso = Some(Curso {
            numero: p.numero,
            ferramenta: p.ferramenta,
            comecou: agora,
            renovado: agora,
            perdeu_desde: None,
            o_que,
        });
        acoes
    }

    /// A estação mandou largar o pedido de número `numero` (ela o interrompeu, ou o teto dela venceu): se é o que está em
    /// curso, o agente solta o canal dele, e ele termina interrompido.
    pub fn largar(&mut self, numero: u64, _agora: Instant) -> Vec<Acao> {
        let mut acoes = Vec::new();
        if self.curso.as_ref().is_some_and(|c| c.numero == numero) {
            self.encerrar(
                Como::Interrompida,
                "a estação mandou largar o pedido",
                &mut acoes,
            );
        }
        acoes
    }

    /// A cada tique: renova o pedido contínuo, e termina a habilidade que chegou, venceu ou perdeu o corpo.
    pub fn tique(&mut self, agora: Instant) -> Vec<Acao> {
        let mut acoes = Vec::new();
        if self.curso.is_none() {
            return acoes;
        }
        let Some(corpo) = self.corpo_recente(agora) else {
            self.encerrar(Como::Interrompida, "o corpo parou de responder", &mut acoes);
            return acoes;
        };
        if !corpo.armado {
            self.encerrar(Como::Interrompida, "o robô foi desarmado", &mut acoes);
            return acoes;
        }
        let renovar = Duration::from_millis(self.tabela.geral.renovar_ms);
        let c = self.curso.as_mut().expect("conferido acima");
        let passou = agora.saturating_duration_since(c.comecou);
        let renova = agora.saturating_duration_since(c.renovado) >= renovar;
        // O desfecho, se a habilidade terminou neste tique.
        let mut fim: Option<(Como, String)> = None;
        match &mut c.o_que {
            OQue::Mover {
                twist,
                metros,
                de,
                por_tempo,
                teto,
            } => {
                let andou = (corpo.posicao[0] - de[0]).hypot(corpo.posicao[1] - de[1]);
                if andou >= *metros - 1e-9 {
                    fim = Some((Como::Cumprida, format!("andei {andou:.2} m")));
                } else if andou < ODOMETRIA_ANDOU_M && passou >= *por_tempo {
                    // O corpo não diz onde está: vale o tempo da distância na velocidade pedida.
                    let por = por_tempo.as_secs_f64();
                    fim = Some((
                        Como::Cumprida,
                        format!("andei por {por:.1} s, sem a posição do corpo"),
                    ));
                } else if passou >= *teto {
                    fim = Some((
                        Como::Vencida,
                        format!("não cheguei: andei {andou:.2} de {metros:.2} m"),
                    ));
                } else if renova {
                    c.renovado = agora;
                    acoes.push(Self::andar(*twist));
                }
            }
            OQue::Olhar {
                dito, teto, taxa, ..
            } => {
                if passou >= *teto {
                    fim = Some((Como::Vencida, format!("não consegui olhar para {dito}")));
                } else if renova && *taxa != [0.0; 2] {
                    c.renovado = agora;
                    acoes.push(Self::torre(*taxa));
                }
            }
            OQue::Procurar {
                dito,
                varredura,
                lado,
                teto,
                sentido,
                ..
            } => {
                // De onde começou, um lado inteiro para lá, dois para cá, dois para lá: a varredura fica centrada.
                let trecho =
                    ((passou.as_secs_f64() / lado.as_secs_f64() + 1.0) / 2.0).floor() as u64;
                let agora_vai = if trecho.is_multiple_of(2) { 1.0 } else { -1.0 };
                if passou >= *teto {
                    fim = Some((Como::Vencida, format!("não achei {dito}")));
                } else if renova || agora_vai != *sentido {
                    *sentido = agora_vai;
                    c.renovado = agora;
                    acoes.push(Self::torre([*varredura * agora_vai, 0.0]));
                }
            }
        }
        if let Some((como, motivo)) = fim {
            let c = self.curso.take().expect("conferido acima");
            acoes.push(Self::soltar(&c.o_que));
            acoes.push(Self::terminou(c.numero, &c.ferramenta, como, motivo));
        }
        acoes
    }

    /// As caixas de uma detecção: é por elas que olhar e procurar sabem do alvo.
    pub fn deteccoes(&mut self, caixas: &[Caixa], agora: Instant) -> Vec<Acao> {
        let mut acoes = Vec::new();
        let Some(palavras) = self.curso.as_ref().and_then(|c| match &c.o_que {
            OQue::Olhar { palavras, .. } | OQue::Procurar { palavras, .. } => {
                Some(palavras.clone())
            }
            OQue::Mover { .. } => None,
        }) else {
            return acoes;
        };
        let centro = self.alvo(&palavras, caixas).map(|c| c.centro);
        let c = self.curso.as_mut().expect("conferido acima");
        let mut fim: Option<String> = None;
        match &mut c.o_que {
            OQue::Procurar { dito, .. } => {
                if centro.is_some() {
                    fim = Some(format!("achei {dito}"));
                }
            }
            OQue::Olhar {
                dito,
                ganho,
                tolerancia,
                precisa,
                seguidas,
                taxa,
                ..
            } => {
                let nova = match centro {
                    // O alvo sumiu do quadro: a torre para onde está, e a contagem recomeça.
                    None => {
                        *seguidas = 0;
                        [0.0; 2]
                    }
                    Some([x, y]) => {
                        let (ex, ey) = (x - 0.5, y - 0.5);
                        if ex.abs() <= *tolerancia && ey.abs() <= *tolerancia {
                            *seguidas += 1;
                            [0.0; 2]
                        } else {
                            *seguidas = 0;
                            // O sinal da DJI: yaw à direita, pitch para cima; no quadro, y cresce para baixo.
                            [*ganho * 2.0 * ex, -*ganho * 2.0 * ey]
                        }
                    }
                };
                if *seguidas >= *precisa {
                    fim = Some(format!("estou olhando para {dito}"));
                } else if nova != *taxa || nova != [0.0; 2] {
                    *taxa = nova;
                    c.renovado = agora;
                    acoes.push(Self::torre(nova));
                }
            }
            OQue::Mover { .. } => {}
        }
        if let Some(motivo) = fim {
            let c = self.curso.take().expect("conferido acima");
            acoes.push(Self::torre([0.0; 2]));
            acoes.push(Self::terminou(
                c.numero,
                &c.ferramenta,
                Como::Cumprida,
                motivo,
            ));
        }
        acoes
    }
}
