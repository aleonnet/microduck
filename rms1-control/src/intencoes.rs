//! O que os clientes pedem. Escrito pelo socket, lido uma vez por ciclo pelo laço. Cada campo é uma
//! troca atômica: nenhum dos lados espera o outro.
//!
//! Giro do corpo e torre são campos separados, com carimbo, porque a pergunta do laço nunca é "qual é o
//! valor", e sim "de quando é". É isso que o prazo lê.
//!
//! **Quem manda.** Cada fonte da tabela de `rms1d.toml` (`[[fontes]]`) tem o seu quadro de pedidos de rodas e de velocidade
//! da torre. A cada leitura, em cada canal, vale o pedido vivo da fonte de maior prioridade; vivo é o que chegou dentro do
//! prazo da fonte. É o desenho do `twist_mux` do ROS, por canal. O pato de hoje só diz quem pediu (`move.source`), e ele
//! mesmo escreve que lá o último que escreve vence. A parada não é arbitrada: [`Intencoes::parar`] vale para qualquer fonte.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;
use rms1d_params::{Canal, FONTE_SEM_NOME, Fonte};

#[derive(Debug, Clone, Copy)]
struct Carimbado<T> {
    valor: T,
    em: Instant,
}

/// Quem segue quem. Nomes da lista de skills do console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modo {
    /// `robot_mode_free`: chassi e torre independentes.
    Livre,
    /// `robot_mode_chassis_follow`, o `gimbal_lead` da DJI.
    ChassiSegueTorre,
    /// `robot_mode_gimbal_follow`, o `chassis_lead` da DJI.
    TorreSegueChassi,
}

impl Modo {
    pub const TODOS: [Modo; 3] = [Modo::Livre, Modo::ChassiSegueTorre, Modo::TorreSegueChassi];

    pub fn nome(self) -> &'static str {
        match self {
            Self::Livre => "modo_livre",
            Self::ChassiSegueTorre => "chassi_segue_torre",
            Self::TorreSegueChassi => "torre_segue_chassi",
        }
    }

    pub fn de_nome(nome: &str) -> Option<Self> {
        Self::TODOS.into_iter().find(|m| m.nome() == nome)
    }

    fn codigo(self) -> u8 {
        match self {
            Self::Livre => 0,
            Self::ChassiSegueTorre => 1,
            Self::TorreSegueChassi => 2,
        }
    }

    fn de_codigo(c: u8) -> Self {
        match c {
            1 => Self::ChassiSegueTorre,
            2 => Self::TorreSegueChassi,
            _ => Self::Livre,
        }
    }
}

/// O pedido de energia: lido uma vez por ciclo, e o último vence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PedidoEnergia {
    Armar,
    Desarmar,
}

const ENERGIA_NADA: u8 = 0;
const ENERGIA_ARMAR: u8 = 1;
const ENERGIA_DESARMAR: u8 = 2;

/// Uma fonte da tabela, pelo lugar dela. Vale para as [`Intencoes`] que a deram ([`Intencoes::fonte`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FonteId(usize);

impl FonteId {
    /// A de quem não diz o nome no `hello`: é sempre a primeira linha.
    pub const SEM_NOME: FonteId = FonteId(0);
}

/// Uma linha da tabela, com o quadro de pedidos dela.
struct Linha {
    nome: Arc<str>,
    prioridade: u8,
    /// Um bit por [`Canal`].
    canais: u8,
    /// O prazo próprio; sem ele vale o de `[seguranca]`.
    prazo: Option<Duration>,
    so_autonomo: bool,
    twist: ArcSwapOption<Carimbado<[f64; 3]>>,
    /// Velocidade da torre, em graus por segundo, com o sinal da DJI: yaw à direita, pitch para cima.
    taxa_torre: ArcSwapOption<Carimbado<[f64; 2]>>,
}

fn bit(canal: Canal) -> u8 {
    1 << canal as u8
}

impl Linha {
    fn nova(f: &Fonte) -> Self {
        Self {
            nome: Arc::from(f.nome.as_str()),
            prioridade: f.prioridade,
            canais: f.canais.iter().fold(0, |m, c| m | bit(*c)),
            prazo: f.prazo_ms.map(Duration::from_millis),
            so_autonomo: f.so_autonomo,
            twist: ArcSwapOption::empty(),
            taxa_torre: ArcSwapOption::empty(),
        }
    }
}

/// O alvo de ângulo da torre: um só para o robô, com quem o pôs e quando.
#[derive(Debug, Clone, Copy)]
struct AlvoTorre {
    /// Yaw e pitch do contrato, em radianos.
    valor: [f64; 2],
    fonte: usize,
    em: Instant,
}

pub struct Intencoes {
    /// A tabela, com a linha de quem não diz o nome na frente. Não muda depois de montada.
    fontes: Vec<Linha>,
    /// `[seguranca] prazo_ms`: o prazo da fonte que não tem um próprio.
    prazo_padrao: Duration,
    /// `None` até alguém pedir: armar não leva a torre a lugar nenhum.
    torre: ArcSwapOption<AlvoTorre>,
    /// O robô por conta própria (`robot.autonomous`): as fontes `so_autonomo` só contam com isto ligado.
    autonomia: AtomicBool,
    energia: AtomicU8,
    modo: AtomicU8,
    armado_pedido: AtomicBool,
    encerrar: AtomicBool,
    /// A sensibilidade das placas pedida pelo cliente (`s1.armor`); `SEM_SENSIBILIDADE` até alguém pedir.
    sensibilidade: AtomicU8,
    /// Há uma sensibilidade pedida que o laço ainda não mandou às placas.
    sensibilidade_nova: AtomicBool,
}

const SEM_SENSIBILIDADE: u8 = u8::MAX;

/// O que o laço lê no começo do ciclo.
#[derive(Debug, Clone, Copy)]
pub struct Leitura {
    pub twist: [f64; 3],
    /// `None` se ninguém nunca pediu movimento.
    pub idade_twist: Option<Duration>,
    /// O prazo próprio da fonte de `twist`; `None` quando ela não tem um: vale o de `[seguranca]`.
    pub prazo_twist: Option<Duration>,
    /// Quem fez o pedido de `twist`, vivo ou vencido.
    pub fonte_twist: Option<FonteId>,
    /// Quem tem as rodas: a fonte de maior prioridade com pedido vivo. `None` sem nenhum.
    pub dono_rodas: Option<FonteId>,
    pub torre: Option<[f64; 2]>,
    pub taxa_torre: [f64; 2],
    /// `None` se ninguém nunca pediu velocidade da torre, ou se quem tem a torre a tem só pelo alvo.
    pub idade_taxa: Option<Duration>,
    /// Como `prazo_twist`, para a velocidade da torre.
    pub prazo_taxa: Option<Duration>,
    /// Quem tem a torre: a fonte de maior prioridade com velocidade viva, ou com um alvo posto dentro do prazo dela.
    pub dono_torre: Option<FonteId>,
    pub modo: Modo,
}

/// Só a fonte sem nome, com todos os canais: o quadro único de antes da tabela. A produção usa [`Intencoes::com_fontes`].
impl Default for Intencoes {
    fn default() -> Self {
        Self::com_fontes(
            &[],
            Duration::from_millis(rms1d_params::Seguranca::default().prazo_ms),
        )
    }
}

impl Intencoes {
    /// Com a tabela de `rms1d.toml` e o prazo de `[seguranca]`. A linha de quem não diz o nome vai para a frente; se a
    /// tabela não a traz (`Params::validar` recusa isso), entra uma com todos os canais, que perde para todas.
    pub fn com_fontes(tabela: &[Fonte], prazo_padrao: Duration) -> Self {
        let sem_nome = tabela
            .iter()
            .find(|f| f.nome == FONTE_SEM_NOME)
            .cloned()
            .unwrap_or_else(|| Fonte {
                nome: FONTE_SEM_NOME.to_owned(),
                prioridade: 0,
                canais: Canal::TODOS.to_vec(),
                prazo_ms: None,
                so_autonomo: false,
            });
        let fontes = std::iter::once(&sem_nome)
            .chain(tabela.iter().filter(|f| f.nome != FONTE_SEM_NOME))
            .map(Linha::nova)
            .collect();
        Self {
            fontes,
            prazo_padrao,
            torre: ArcSwapOption::empty(),
            autonomia: AtomicBool::new(false),
            energia: AtomicU8::new(ENERGIA_NADA),
            modo: AtomicU8::new(Modo::Livre.codigo()),
            armado_pedido: AtomicBool::new(false),
            encerrar: AtomicBool::new(false),
            sensibilidade: AtomicU8::new(SEM_SENSIBILIDADE),
            sensibilidade_nova: AtomicBool::new(false),
        }
    }

    fn linha(&self, fonte: FonteId) -> &Linha {
        self.fontes.get(fonte.0).unwrap_or(&self.fontes[0])
    }

    /// A fonte de um nome dito no `hello`. Nome ausente, ou que a tabela não tem, é a fonte sem nome.
    pub fn fonte(&self, nome: Option<&str>) -> FonteId {
        FonteId(
            nome.and_then(|n| self.fontes.iter().position(|f| &*f.nome == n))
                .unwrap_or(0),
        )
    }

    pub fn nome(&self, fonte: FonteId) -> Arc<str> {
        self.linha(fonte).nome.clone()
    }

    /// A tabela dá este canal à fonte.
    pub fn pode(&self, fonte: FonteId, canal: Canal) -> bool {
        self.linha(fonte).canais & bit(canal) != 0
    }

    /// A fonte é das que só valem com a autonomia ligada, e ela está desligada.
    pub fn espera_a_autonomia(&self, fonte: FonteId) -> bool {
        self.linha(fonte).so_autonomo && !self.autonomia()
    }

    /// O pedido desta fonte neste canal conta agora: a tabela dá o canal, e a autonomia não a segura.
    fn conta(&self, f: &Linha, canal: Canal) -> bool {
        f.canais & bit(canal) != 0 && !(f.so_autonomo && !self.autonomia())
    }

    fn prazo(&self, f: &Linha) -> Duration {
        f.prazo.unwrap_or(self.prazo_padrao)
    }

    /// A fonte tem a torre agora: velocidade viva, ou um alvo posto por ela dentro do prazo.
    fn tem_a_torre(&self, i: usize, alvo: Option<&AlvoTorre>, agora: Instant) -> bool {
        let f = &self.fontes[i];
        if !self.conta(f, Canal::Torre) {
            return false;
        }
        let prazo = self.prazo(f);
        let vivo = |em: Instant| agora.saturating_duration_since(em) <= prazo;
        f.taxa_torre.load().as_ref().is_some_and(|c| vivo(c.em))
            || alvo.is_some_and(|a| a.fonte == i && vivo(a.em))
    }

    /// A fonte de prioridade maior que tem este canal agora, se há uma: é para ela que `fonte` perde. Só as rodas e a
    /// torre são disputadas; nos outros canais ninguém supera ninguém.
    pub fn quem_supera(&self, fonte: FonteId, canal: Canal, agora: Instant) -> Option<FonteId> {
        let minha = self.linha(fonte).prioridade;
        let alvo = self.torre.load_full();
        (0..self.fontes.len())
            .filter(|i| self.fontes[*i].prioridade > minha)
            .filter(|i| match canal {
                Canal::Rodas => {
                    let f = &self.fontes[*i];
                    self.conta(f, Canal::Rodas)
                        && f.twist
                            .load()
                            .as_ref()
                            .is_some_and(|c| agora.saturating_duration_since(c.em) <= self.prazo(f))
                }
                Canal::Torre => self.tem_a_torre(*i, alvo.as_deref(), agora),
                _ => false,
            })
            .max_by_key(|i| self.fontes[*i].prioridade)
            .map(FonteId)
    }

    /// `false` se o pedido não foi guardado: a tabela não dá as rodas à fonte, ou ela espera a autonomia.
    pub fn pedir_twist(&self, fonte: FonteId, twist: [f64; 3]) -> bool {
        let f = self.linha(fonte);
        if !self.conta(f, Canal::Rodas) {
            return false;
        }
        f.twist.store(Some(Arc::new(Carimbado {
            valor: twist,
            em: Instant::now(),
        })));
        true
    }

    /// O alvo de ângulo é um só. `false` se não foi trocado: a fonte não tem a torre na tabela, espera a autonomia, ou uma
    /// de prioridade maior tem a torre agora.
    pub fn pedir_torre(&self, fonte: FonteId, yaw: f64, pitch: f64) -> bool {
        let agora = Instant::now();
        if !self.conta(self.linha(fonte), Canal::Torre)
            || self.quem_supera(fonte, Canal::Torre, agora).is_some()
        {
            return false;
        }
        self.torre.store(Some(Arc::new(AlvoTorre {
            valor: [yaw, pitch],
            fonte: fonte.0.min(self.fontes.len() - 1),
            em: agora,
        })));
        true
    }

    /// Mover a torre por velocidade esquece o alvo de ângulo: um alvo velho a puxaria de volta ao parar. Só quem tem a
    /// torre o apaga: o pedido de uma fonte superada fica guardado, e não mexe no alvo de quem manda.
    pub fn pedir_taxa_torre(&self, fonte: FonteId, yaw: f64, pitch: f64) -> bool {
        let f = self.linha(fonte);
        if !self.conta(f, Canal::Torre) {
            return false;
        }
        let agora = Instant::now();
        if (yaw != 0.0 || pitch != 0.0) && self.quem_supera(fonte, Canal::Torre, agora).is_none() {
            self.torre.store(None);
        }
        f.taxa_torre.store(Some(Arc::new(Carimbado {
            valor: [yaw, pitch],
            em: agora,
        })));
        true
    }

    fn esquecer_pedidos(&self) {
        for f in &self.fontes {
            f.twist.store(None);
            f.taxa_torre.store(None);
        }
    }

    /// A parada nunca é barrada: vale de qualquer fonte, e apaga os pedidos de rodas e de velocidade da torre de todas.
    /// Fica o zero de quem parou, como um pedido novo dela; quem tem prioridade volta a mandar no pedido seguinte.
    pub fn parar(&self, fonte: FonteId) {
        self.esquecer_pedidos();
        self.pedir_twist(fonte, [0.0; 3]);
        self.pedir_taxa_torre(fonte, 0.0, 0.0);
    }

    /// O robô por conta própria, ligado ou desligado. Desligado, o que as fontes `so_autonomo` pediram deixa de valer:
    /// as rodas, a velocidade da torre, e o alvo de ângulo, se foi uma delas que o pôs (senão a torre seguiria indo até
    /// ele com a autonomia desligada).
    pub fn definir_autonomia(&self, ligada: bool) {
        self.autonomia.store(ligada, Ordering::Relaxed);
        if !ligada {
            for f in self.fontes.iter().filter(|f| f.so_autonomo) {
                f.twist.store(None);
                f.taxa_torre.store(None);
            }
            let alvo = self.torre.load_full();
            if alvo
                .as_ref()
                .is_some_and(|a| self.fontes[a.fonte].so_autonomo)
            {
                self.torre.compare_and_swap(&alvo, None);
            }
        }
    }

    pub fn autonomia(&self) -> bool {
        self.autonomia.load(Ordering::Relaxed)
    }

    pub fn pedir_modo(&self, modo: Modo) {
        self.modo.store(modo.codigo(), Ordering::Relaxed);
    }

    pub fn modo(&self) -> Modo {
        Modo::de_codigo(self.modo.load(Ordering::Relaxed))
    }

    /// Ao armar, o que foi pedido antes não vale, de fonte nenhuma: ligar não move o robô, nem com um pedido de meio
    /// segundo atrás. É aqui, e não no laço, para o pedido feito logo depois da resposta valer.
    pub fn pedir_armar(&self, sim: bool) {
        let antes = self.armado_pedido.swap(sim, Ordering::Relaxed);
        if sim && !antes {
            self.esquecer_pedidos();
            self.torre.store(None);
        }
        self.energia.store(
            if sim { ENERGIA_ARMAR } else { ENERGIA_DESARMAR },
            Ordering::Relaxed,
        );
    }

    /// O que o último cliente pediu, para a alternância de `robot.enable`.
    pub fn armado_pedido(&self) -> bool {
        self.armado_pedido.load(Ordering::Relaxed)
    }

    /// O processo vai sair: o laço para o robô, descarrega a gravação e termina.
    pub fn pedir_encerrar(&self) {
        self.encerrar.store(true, Ordering::Relaxed);
    }

    pub fn encerrando(&self) -> bool {
        self.encerrar.load(Ordering::Relaxed)
    }

    /// A sensibilidade das placas, de 0 a 10, como a DJI: o laço a manda às seis no próximo ciclo com o barramento vivo. Quem
    /// chama valida a faixa.
    pub fn pedir_sensibilidade(&self, nivel: u8) {
        self.sensibilidade.store(nivel, Ordering::Relaxed);
        self.sensibilidade_nova.store(true, Ordering::Relaxed);
    }

    /// A última sensibilidade pedida pelo cliente, se houve.
    pub fn sensibilidade(&self) -> Option<u8> {
        Some(self.sensibilidade.load(Ordering::Relaxed)).filter(|n| *n != SEM_SENSIBILIDADE)
    }

    /// A sensibilidade nova, uma vez: lida pelo laço.
    pub fn tomar_sensibilidade(&self) -> Option<u8> {
        if self.sensibilidade_nova.swap(false, Ordering::Relaxed) {
            self.sensibilidade()
        } else {
            None
        }
    }

    pub fn tomar_energia(&self) -> Option<PedidoEnergia> {
        match self.energia.swap(ENERGIA_NADA, Ordering::Relaxed) {
            ENERGIA_ARMAR => Some(PedidoEnergia::Armar),
            ENERGIA_DESARMAR => Some(PedidoEnergia::Desarmar),
            _ => None,
        }
    }

    /// O que vale agora. Em cada canal, o pedido vivo da fonte de maior prioridade; sem pedido vivo de ninguém, o mais
    /// recente, já vencido, para o estado seguir dizendo que o prazo passou. Quem lê julga a idade com o mesmo prazo.
    pub fn ler(&self, agora: Instant) -> Leitura {
        let idade = |em: Instant| agora.saturating_duration_since(em);
        // As rodas.
        let mut dono: Option<(usize, Arc<Carimbado<[f64; 3]>>)> = None;
        let mut recente: Option<(usize, Arc<Carimbado<[f64; 3]>>)> = None;
        for (i, f) in self.fontes.iter().enumerate() {
            if !self.conta(f, Canal::Rodas) {
                continue;
            }
            let Some(c) = f.twist.load_full() else {
                continue;
            };
            if idade(c.em) <= self.prazo(f)
                && dono
                    .as_ref()
                    .is_none_or(|(j, _)| f.prioridade > self.fontes[*j].prioridade)
            {
                dono = Some((i, c.clone()));
            }
            if recente.as_ref().is_none_or(|(_, r)| c.em > r.em) {
                recente = Some((i, c));
            }
        }
        let dono_rodas = dono.as_ref().map(|(i, _)| FonteId(*i));
        let rodas = dono.or(recente);

        // A torre: tem-na a fonte de maior prioridade com velocidade viva, ou com o alvo posto dentro do prazo.
        let mut alvo = self.torre.load_full();
        let dono_torre = (0..self.fontes.len())
            .filter(|i| self.tem_a_torre(*i, alvo.as_deref(), agora))
            .max_by_key(|i| self.fontes[*i].prioridade);
        let taxa = match dono_torre {
            // Quem a tem só pelo alvo não tem velocidade: a torre vai ao alvo.
            Some(i) => self.fontes[i]
                .taxa_torre
                .load_full()
                .filter(|c| idade(c.em) <= self.prazo(&self.fontes[i]))
                .map(|c| (i, c)),
            None => self
                .fontes
                .iter()
                .enumerate()
                .filter(|(_, f)| self.conta(f, Canal::Torre))
                .filter_map(|(i, f)| f.taxa_torre.load_full().map(|c| (i, c)))
                .max_by_key(|(_, c)| c.em),
        };
        // Uma velocidade que passa a valer depois de o alvo ter sido posto o apaga, como no pedido: é o caso da fonte
        // superada que pediu velocidade e herdou a torre quando a posse da outra venceu. Sem isto o alvo velho voltaria.
        if let (Some(a), Some((_, c))) = (&alvo, &taxa)
            && dono_torre.is_some()
            && c.valor != [0.0; 2]
            && a.em <= c.em
        {
            self.torre.compare_and_swap(&alvo, None);
            alvo = None;
        }
        Leitura {
            twist: rodas.as_ref().map_or([0.0; 3], |(_, c)| c.valor),
            idade_twist: rodas.as_ref().map(|(_, c)| idade(c.em)),
            prazo_twist: rodas.as_ref().and_then(|(i, _)| self.fontes[*i].prazo),
            fonte_twist: rodas.as_ref().map(|(i, _)| FonteId(*i)),
            dono_rodas,
            torre: alvo.map(|a| a.valor),
            taxa_torre: taxa.as_ref().map_or([0.0; 2], |(_, c)| c.valor),
            idade_taxa: taxa.as_ref().map(|(_, c)| idade(c.em)),
            prazo_taxa: taxa.as_ref().and_then(|(i, _)| self.fontes[*i].prazo),
            dono_torre: dono_torre.map(FonteId),
            modo: self.modo(),
        }
    }
}
