//! A telemetria do corpo: assinar como a DJI, responder a cada pacote e guardar o que chega.
//!
//! **A sequência**, a mesma da gravação da DJI: observa quais mensagens estão no ar, cancela cada uma, registra o nó, assina,
//! e recebe. O controlador não aceita o mesmo dado duas vezes pelo mesmo nó, e por isso cancelar vem antes.
//!
//! **Recusa não se repete:** o código é a resposta, e repetir no robô é tentativa e erro. Sem resposta, ela recomeça só com
//! quadros chegando, e com intervalo. Pacotes que param recomeçam a sequência, que é o caso de o corpo simulado recarregar.
//!
//! **Só decodifica o que assinou:** antes disso a ordem não é conhecida, e nada é adivinhado pelos valores.
//!
//! **Convenção:** o corpo fala com x à frente, y à direita e z para baixo. O contrato usa y à esquerda e z para cima. A
//! conversão fica só em [`Medidas::de`].

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use rms1_proto::Quadro;
use rms1_proto::barramento::{
    self, ASSINATURA, CONTROLADOR, DADO_JA_ASSINADO, FREQUENCIA_HZ, Layout, MENSAGEM,
    MENSAGEM_JA_EXISTE, MENSAGEM_NAO_EXISTE, NO_ASSINANTE, NO_JA_EXISTE, OK, Pacote, Pedido,
    VERSAO_NAO_CONFERE, id,
};
use rms1d_params::Params;

/// Os bits do estado do chassi, na ordem do firmware (`biblioteca/guia/telemetria.md`).
pub const BITS_CHASSI: [&str; 11] = [
    "static",
    "uphill",
    "downhill",
    "on_slope",
    "pick_up",
    "slip",
    "impact_x",
    "impact_y",
    "impact_z",
    "roll_over",
    "hill_static",
];
pub const BIT_LEVANTADO: u32 = 1 << 4;
pub const BIT_CAPOTOU: u32 = 1 << 9;
pub const PLACAS: usize = 6;
const GRAUS_POR_VOLTA: f64 = 360.0;
/// O ângulo das rodas: de 0 a 32767, uma volta (SDK e gravação).
const ANGULO_POR_VOLTA: f64 = 32768.0;

/// Em que passo a assinatura está.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Passo {
    Cancelar,
    Registrar,
    Assinar,
}

impl Passo {
    pub fn nome(self) -> &'static str {
        match self {
            Self::Cancelar => "cancelar",
            Self::Registrar => "registrar",
            Self::Assinar => "assinar",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enlace {
    /// O corpo falso não devolve nada, e aqui não se assina.
    Desligado,
    Observando,
    Cancelando,
    Registrando,
    Assinando,
    /// Assinou, e o primeiro pacote ainda não chegou.
    Esperando,
    Recebendo,
    Recusado {
        passo: Passo,
        codigo: u8,
    },
    SemResposta {
        passo: Passo,
    },
}

impl Enlace {
    pub fn nome(self) -> &'static str {
        match self {
            Self::Desligado => "off",
            Self::Observando => "observing",
            Self::Cancelando => "cancelling",
            Self::Registrando => "registering",
            Self::Assinando => "subscribing",
            Self::Esperando => "waiting",
            Self::Recebendo => "receiving",
            Self::Recusado { .. } => "refused",
            Self::SemResposta { .. } => "no_answer",
        }
    }
}

/// Um pacote nas unidades e nos eixos do contrato: x à frente, y à esquerda, z para cima.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Medidas {
    pub velocidade_solo: [f64; 3],
    pub velocidade_corpo: [f64; 3],
    /// rpm com sinal de andar para a frente, na ordem frente-direita, frente-esquerda, trás-esquerda, trás-direita.
    pub rodas_rpm: [f64; 4],
    pub rodas_angulo_grau: [f64; 4],
    pub rodas_estado: [u8; 4],
    pub posicao: [f64; 3],
    /// Em g: +1 em z parado, como um acelerômetro apoiado.
    pub aceleracao: [f64; 3],
    pub giro: [f64; 3],
    /// Graus, positivo à esquerda.
    pub rumo: f64,
    /// Graus, como o corpo manda. **Sinal sem prova:** as gravações não têm inclinação suficiente.
    pub inclinacao: f64,
    pub rolagem: f64,
    pub volts: f64,
    pub percentual: f64,
    pub corrente_a: f64,
    pub temperatura_c: f64,
    pub relogio_ms: u32,
}

fn eixos(v: [f32; 3]) -> [f64; 3] {
    [f64::from(v[0]), -f64::from(v[1]), -f64::from(v[2])]
}

impl Medidas {
    pub fn de(p: &Pacote) -> Self {
        let r = p.rodas.rotacao.map(f64::from);
        Self {
            velocidade_solo: eixos(p.velocidade_solo),
            velocidade_corpo: eixos(p.velocidade_corpo),
            // No fio, a roda 2 e a 3 vêm invertidas, como em `0x3f/0x20` (medido ao girar, `hdvt_gimbal_reads.strace`).
            rodas_rpm: [r[0], -r[1], -r[2], r[3]],
            rodas_angulo_grau: p
                .rodas
                .angulo
                .map(|a| f64::from(a) * GRAUS_POR_VOLTA / ANGULO_POR_VOLTA),
            rodas_estado: p.rodas.estado,
            posicao: eixos(p.posicao),
            aceleracao: eixos(p.aceleracao),
            giro: eixos(p.giro),
            rumo: -f64::from(p.atitude[0]),
            inclinacao: f64::from(p.atitude[1]),
            rolagem: f64::from(p.atitude[2]),
            volts: f64::from(p.bateria.milivolts) / 1000.0,
            percentual: f64::from(p.bateria.percentual),
            corrente_a: f64::from(p.bateria.corrente) / 1000.0,
            temperatura_c: f64::from(p.bateria.temperatura) / 10.0,
            relogio_ms: p.tempo_ms,
        }
    }

    /// O quaternion tronco para mundo, `[w, x, y, z]`, nos eixos do contrato.
    pub fn quaternion(&self) -> [f64; 4] {
        let (y, p, r) = (
            self.rumo.to_radians() / 2.0,
            -self.inclinacao.to_radians() / 2.0,
            self.rolagem.to_radians() / 2.0,
        );
        let (cy, sy, cp, sp, cr, sr) = (y.cos(), y.sin(), p.cos(), p.sin(), r.cos(), r.sin());
        [
            cr * cp * cy + sr * sp * sy,
            sr * cp * cy - cr * sp * sy,
            cr * sp * cy + sr * cp * sy,
            cr * cp * sy - sr * sp * cy,
        ]
    }

    /// A gravidade no tronco, em g: `[0, 0, -1]` em pé, como o contrato pede.
    pub fn gravidade(&self) -> [f64; 3] {
        self.aceleracao.map(|a| -a)
    }
}

/// A torre, em graus: rumo positivo à esquerda, inclinação positiva para cima.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Torre {
    pub rumo: f64,
    pub inclinacao: f64,
    pub rumo_solo: f64,
    pub inclinacao_solo: f64,
}

/// Uma placa de armadura: contador, para assinante lento não perder toque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Placa {
    pub toques: u64,
    pub tipo: u8,
    pub microfone: u16,
    pub aceleracao: u16,
}

/// O retrato do que se sabe do corpo. O que nunca chegou fica `None`, e nunca vira zero.
#[derive(Debug, Clone, PartialEq)]
pub struct Amostra {
    pub enlace: Enlace,
    pub pacotes: u64,
    pub ignorados: u64,
    pub medidas: Option<Medidas>,
    pub torre: Option<Torre>,
    pub estado_chassi: u32,
    /// Vezes que cada bit ligou.
    pub contagem_chassi: [u64; BITS_CHASSI.len()],
    /// Índice 0 é a placa 1.
    pub placas: [Placa; PLACAS],
    /// Topo esquerda, topo direita.
    pub infravermelho: [u64; 2],
}

/// A sequência e o estado. Sem E/S: quem chama entrega o que chegou, e envia os pedidos.
pub struct Assinante {
    amostra: Amostra,
    layout: Layout,
    ligado: bool,
    janela: Duration,
    espera: Duration,
    tentativas: u32,
    recomecar_parado: Duration,
    recomecar_sem_resposta: Duration,
    inicio: Option<Instant>,
    vistas: BTreeSet<u8>,
    a_cancelar: Vec<u8>,
    cancelando: Option<u8>,
    enviado_em: Option<Instant>,
    enviadas: u32,
    ultimo_pacote: Option<Instant>,
    ultimo_quadro: Option<Instant>,
    desistiu_em: Option<Instant>,
    /// Já cancelou a nossa mensagem por ouvir que ela existia: na segunda vez, é recusa.
    cancelou_a_nossa: bool,
    mudou: bool,
}

impl Assinante {
    pub fn new(p: &Params) -> Self {
        let t = &p.telemetria;
        let ligado = p.corpo.tipo != rms1d_params::TipoCorpo::Falso;
        Self {
            amostra: Amostra {
                enlace: if ligado {
                    Enlace::Observando
                } else {
                    Enlace::Desligado
                },
                pacotes: 0,
                ignorados: 0,
                medidas: None,
                torre: None,
                estado_chassi: 0,
                contagem_chassi: [0; BITS_CHASSI.len()],
                placas: [Placa::default(); PLACAS],
                infravermelho: [0; 2],
            },
            layout: Layout::da_assinatura(),
            ligado,
            janela: Duration::from_millis(t.janela_ms),
            espera: Duration::from_millis(t.espera_ms),
            tentativas: t.tentativas,
            recomecar_parado: Duration::from_millis(t.recomecar_parado_ms),
            recomecar_sem_resposta: Duration::from_millis(t.recomecar_sem_resposta_ms),
            inicio: None,
            vistas: BTreeSet::new(),
            a_cancelar: Vec::new(),
            cancelando: None,
            enviado_em: None,
            enviadas: 0,
            ultimo_pacote: None,
            ultimo_quadro: None,
            desistiu_em: None,
            cancelou_a_nossa: false,
            mudou: true,
        }
    }

    pub fn amostra(&self) -> &Amostra {
        &self.amostra
    }

    pub fn idade_do_pacote(&self, agora: Instant) -> Option<Duration> {
        self.ultimo_pacote
            .map(|t| agora.saturating_duration_since(t))
    }

    /// Algo mudou desde a última vez: é o que evita publicar o mesmo retrato a cada ciclo.
    pub fn tomar_mudanca(&mut self) -> bool {
        std::mem::take(&mut self.mudou)
    }

    fn ir(&mut self, e: Enlace) {
        if self.amostra.enlace != e {
            tracing::info!(
                de = self.amostra.enlace.nome(),
                para = e.nome(),
                "telemetria"
            );
        }
        self.amostra.enlace = e;
        self.mudou = true;
    }

    fn pedir(&mut self, agora: Instant, p: Pedido, saida: &mut Vec<Pedido>) {
        saida.push(p);
        self.enviado_em = Some(agora);
        self.enviadas += 1;
    }

    /// Cancela o que falta, e depois assina: o nó já foi registrado antes de cancelar, como a DJI faz.
    fn proximo_cancelamento(&mut self, agora: Instant, saida: &mut Vec<Pedido>) {
        self.cancelando = self.a_cancelar.pop();
        if let Some(mensagem) = self.cancelando {
            self.enviadas = 0;
            self.ir(Enlace::Cancelando);
            self.pedir(agora, Pedido::Cancelar { mensagem }, saida);
        } else {
            self.assinar(agora, saida);
        }
    }

    fn registrar(&mut self, agora: Instant, saida: &mut Vec<Pedido>) {
        self.enviadas = 0;
        self.ir(Enlace::Registrando);
        self.pedir(agora, Pedido::RegistrarNo, saida);
    }

    fn assinar(&mut self, agora: Instant, saida: &mut Vec<Pedido>) {
        self.enviadas = 0;
        self.ir(Enlace::Assinando);
        self.pedir(agora, pedido_de_assinatura(), saida);
    }

    fn recomecar(&mut self, agora: Instant) {
        self.vistas.clear();
        self.a_cancelar.clear();
        self.cancelando = None;
        self.enviado_em = None;
        self.enviadas = 0;
        self.inicio = Some(agora);
        self.desistiu_em = None;
        self.cancelou_a_nossa = false;
        self.ir(Enlace::Observando);
    }

    /// Uma vez por ciclo: os prazos.
    pub fn ciclo(&mut self, agora: Instant, saida: &mut Vec<Pedido>) {
        if !self.ligado {
            return;
        }
        match self.amostra.enlace {
            Enlace::Observando => {
                let inicio = *self.inicio.get_or_insert(agora);
                if agora.saturating_duration_since(inicio) >= self.janela {
                    // Registrar antes de cancelar, como a DJI com o robô em funcionamento (`hdvt_exec_writes.strace`).
                    // Cancelar primeiro deu código 2 ao registrar, no robô, em 2026-11-03.
                    self.a_cancelar = self.vistas.iter().rev().copied().collect();
                    self.registrar(agora, saida);
                }
            }
            Enlace::Cancelando | Enlace::Registrando | Enlace::Assinando => {
                self.reenviar_ou_desistir(agora, saida)
            }
            Enlace::Esperando => {
                if self
                    .enviado_em
                    .is_some_and(|t| agora.saturating_duration_since(t) >= self.recomecar_parado)
                {
                    tracing::warn!("assinou, e nenhum pacote chegou: recomeçando a assinatura");
                    self.recomecar(agora);
                }
            }
            Enlace::Recebendo => {
                if self
                    .idade_do_pacote(agora)
                    .is_some_and(|i| i >= self.recomecar_parado)
                {
                    tracing::warn!("telemetria parada: recomeçando a assinatura");
                    self.recomecar(agora);
                }
            }
            Enlace::SemResposta { .. } => {
                // Só com alguém do outro lado, e sem martelar.
                let tempo = self.desistiu_em.is_none_or(|d| {
                    agora.saturating_duration_since(d) >= self.recomecar_sem_resposta
                });
                let vivo = self
                    .ultimo_quadro
                    .is_some_and(|q| agora.saturating_duration_since(q) < self.espera);
                if tempo && vivo {
                    self.recomecar(agora);
                }
            }
            Enlace::Desligado | Enlace::Recusado { .. } => {}
        }
    }

    fn passo_atual(&self) -> Option<Passo> {
        match self.amostra.enlace {
            Enlace::Cancelando => Some(Passo::Cancelar),
            Enlace::Registrando => Some(Passo::Registrar),
            Enlace::Assinando => Some(Passo::Assinar),
            _ => None,
        }
    }

    fn reenviar_ou_desistir(&mut self, agora: Instant, saida: &mut Vec<Pedido>) {
        if self
            .enviado_em
            .is_some_and(|t| agora.saturating_duration_since(t) < self.espera)
        {
            return;
        }
        let Some(passo) = self.passo_atual() else {
            return;
        };
        if self.enviadas >= self.tentativas {
            // Cancelar sem resposta não trava: a mensagem pode nem existir, e o registro é quem diz se há alguém.
            if passo == Passo::Cancelar {
                self.proximo_cancelamento(agora, saida);
                return;
            }
            tracing::warn!(
                passo = passo.nome(),
                "o controlador não respondeu à assinatura"
            );
            self.desistiu_em = Some(agora);
            self.ir(Enlace::SemResposta { passo });
            return;
        }
        let p = match passo {
            Passo::Registrar => Pedido::RegistrarNo,
            Passo::Assinar => pedido_de_assinatura(),
            Passo::Cancelar => match self.cancelando {
                Some(mensagem) => Pedido::Cancelar { mensagem },
                None => return,
            },
        };
        self.pedir(agora, p, saida);
    }

    /// Um quadro que chegou. Devolve em `saida` o que ele pede, inclusive a resposta ao pacote.
    pub fn receber(&mut self, q: &Quadro, agora: Instant, saida: &mut Vec<Pedido>) {
        self.ultimo_quadro = Some(agora);
        let cmd = (q.cmd_set, q.cmd_id);
        if !q.resposta && q.destino == NO_ASSINANTE && cmd == id::PACOTE {
            self.pacote(q, agora, saida);
            return;
        }
        if q.resposta && q.origem == CONTROLADOR && q.destino == NO_ASSINANTE && self.ligado {
            self.resposta(q, cmd, agora, saida);
            return;
        }
        match cmd {
            id::ESTADO_CHASSI => match barramento::ler_estado_chassi(&q.dados) {
                Ok(e) => self.estado(e),
                Err(_) => self.amostra.ignorados += 1,
            },
            id::GOLPE => match barramento::ler_golpe(&q.dados) {
                Ok(g) if (1..=PLACAS as u8).contains(&g.placa) => {
                    let p = &mut self.amostra.placas[usize::from(g.placa - 1)];
                    p.toques += 1;
                    p.tipo = g.tipo;
                    p.microfone = g.microfone;
                    p.aceleracao = g.aceleracao;
                    self.mudou = true;
                }
                _ => self.amostra.ignorados += 1,
            },
            id::INFRAVERMELHO => match barramento::ler_infravermelho(&q.dados) {
                Ok(b) => {
                    for (i, c) in self.amostra.infravermelho.iter_mut().enumerate() {
                        if b & (1 << i) != 0 {
                            *c += 1;
                            self.mudou = true;
                        }
                    }
                }
                Err(_) => self.amostra.ignorados += 1,
            },
            id::TORRE_POSICAO => match barramento::ler_torre_posicao(&q.dados) {
                Ok(t) => {
                    let g = |v: i16| f64::from(v) / 10.0;
                    self.amostra.torre = Some(Torre {
                        rumo: -g(t.rumo),
                        inclinacao: g(t.inclinacao),
                        rumo_solo: -g(t.rumo_solo),
                        inclinacao_solo: g(t.inclinacao_solo),
                    });
                    self.mudou = true;
                }
                Err(_) => self.amostra.ignorados += 1,
            },
            _ => {}
        }
    }

    fn estado(&mut self, novo: u32) {
        let subiram = novo & !self.amostra.estado_chassi;
        for (i, c) in self.amostra.contagem_chassi.iter_mut().enumerate() {
            if subiram & (1 << i) != 0 {
                *c += 1;
            }
        }
        if novo != self.amostra.estado_chassi {
            self.mudou = true;
        }
        self.amostra.estado_chassi = novo;
    }

    fn pacote(&mut self, q: &Quadro, agora: Instant, saida: &mut Vec<Pedido>) {
        // A DJI responde a todos, inclusive aos da assinatura anterior: custa 0,8% da serial, e iguala o que ela faz.
        if self.ligado {
            saida.push(Pedido::Responder { seq: q.seq });
        }
        let Some(mensagem) = barramento::mensagem_do_pacote(&q.dados) else {
            self.amostra.ignorados += 1;
            return;
        };
        if self.amostra.enlace == Enlace::Observando {
            self.vistas.insert(mensagem);
        }
        // Só a mensagem que nós assinamos, e só depois de assinar: antes disso a ordem não é conhecida.
        let nossa = matches!(self.amostra.enlace, Enlace::Esperando | Enlace::Recebendo)
            && mensagem == MENSAGEM;
        if !nossa {
            return;
        }
        match Pacote::ler(&q.dados, &self.layout) {
            Ok(p) => {
                self.amostra.medidas = Some(Medidas::de(&p));
                self.amostra.pacotes += 1;
                self.ultimo_pacote = Some(agora);
                self.estado(p.estado);
                self.mudou = true;
                if self.amostra.enlace != Enlace::Recebendo {
                    self.ir(Enlace::Recebendo);
                }
            }
            Err(e) => {
                self.amostra.ignorados += 1;
                tracing::debug!(erro = %e, "pacote ignorado");
            }
        }
    }

    fn resposta(&mut self, q: &Quadro, cmd: (u8, u8), agora: Instant, saida: &mut Vec<Pedido>) {
        let Ok(codigo) = barramento::ler_codigo(&q.dados) else {
            self.amostra.ignorados += 1;
            return;
        };
        match (self.amostra.enlace, cmd) {
            (Enlace::Cancelando, id::CANCELAR) => {
                if codigo != OK && codigo != MENSAGEM_NAO_EXISTE {
                    tracing::warn!(
                        codigo,
                        sentido = barramento::sentido_do_codigo(codigo),
                        "cancelar recusado"
                    );
                }
                self.proximo_cancelamento(agora, saida);
            }
            (Enlace::Registrando, id::REGISTRAR) => {
                if codigo == VERSAO_NAO_CONFERE {
                    // A resposta deste controlador ao registro, 3 de 3 vezes no robô; a DJI segue. A assinatura diz se o nó existe.
                    tracing::info!(
                        "registro com versão diferente, como a DJI recebe: seguindo para a assinatura"
                    );
                }
                if codigo == OK || codigo == NO_JA_EXISTE || codigo == VERSAO_NAO_CONFERE {
                    self.proximo_cancelamento(agora, saida);
                } else {
                    self.recusa(Passo::Registrar, codigo);
                }
            }
            (Enlace::Assinando, id::ASSINAR) => {
                // Reenviada, a primeira pode ter valido e só a resposta se perdeu: o controlador diz então que já existe.
                let ja_valeu = self.enviadas > 1
                    && (codigo == MENSAGEM_JA_EXISTE || codigo == DADO_JA_ASSINADO);
                if codigo == OK || ja_valeu {
                    self.enviado_em = Some(agora);
                    self.ir(Enlace::Esperando);
                } else if codigo == MENSAGEM_JA_EXISTE && !self.cancelou_a_nossa {
                    // A mensagem existia calada durante a observação, e o código diz qual é: cancela essa, uma vez, e assina de novo.
                    tracing::warn!(
                        "a mensagem que assinamos já existia, calada: cancelando e assinando de novo"
                    );
                    self.cancelou_a_nossa = true;
                    self.a_cancelar = vec![MENSAGEM];
                    self.proximo_cancelamento(agora, saida);
                } else {
                    self.recusa(Passo::Assinar, codigo);
                }
            }
            _ => {}
        }
    }

    fn recusa(&mut self, passo: Passo, codigo: u8) {
        tracing::error!(
            passo = passo.nome(),
            codigo,
            sentido = barramento::sentido_do_codigo(codigo),
            "o controlador recusou a assinatura"
        );
        self.ir(Enlace::Recusado { passo, codigo });
    }
}

/// O sentido de um código de resposta, para quem está acima deste crate.
pub fn sentido(codigo: u8) -> &'static str {
    barramento::sentido_do_codigo(codigo)
}

fn pedido_de_assinatura() -> Pedido {
    Pedido::Assinar {
        mensagem: MENSAGEM,
        dados: &ASSINATURA,
        frequencia: FREQUENCIA_HZ,
    }
}
