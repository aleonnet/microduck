//! Um ciclo: ler intenções, pedido de energia, segurança, ler o corpo, publicar. O ritmo é de quem chama,
//! com `MissedTickBehavior::Skip`, e fica no processo.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use rms1_proto::Leitor;
use rms1_proto::barramento::Pedido;
use rms1d_params::Params;

use crate::corpo::Corpo;
use crate::intencoes::{Intencoes, Modo, PedidoEnergia};
use crate::saude::Saude;
use crate::seguranca::Seguranca;
use crate::telemetria::{Amostra, Assinante, BIT_LEVANTADO};

/// O estado de um ciclo. Montado só se alguém está assinando.
#[derive(Debug, Clone, PartialEq)]
pub struct Estado {
    pub t: f64,
    pub pedido: [f64; 3],
    pub aplicado: [f64; 3],
    pub limitado_por: Vec<&'static str>,
    /// O último alvo aplicado, depois do limite, e não o pedido.
    pub torre: Option<[f64; 2]>,
    pub modo: Modo,
    pub armado: bool,
    pub hz: f64,
    pub perdidos: u64,
    /// O corpo, como chegou; `None` no corpo falso, ou antes de publicar.
    pub corpo: Option<Arc<Amostra>>,
    /// O último pacote é recente: sem isto, o que está em `corpo` é passado, e não pode parecer vivo.
    pub corpo_recente: bool,
    /// Há quantos milissegundos chegou o último pacote.
    pub idade_pacote_ms: Option<u64>,
}

pub struct Laco<C: Corpo> {
    seguranca: Seguranca<C>,
    intencoes: Arc<Intencoes>,
    saude: Arc<Saude>,
    leitor: Leitor,
    assinante: Assinante,
    pedidos: Vec<Pedido>,
    idade_maxima_pacote: Duration,
    caixa: Vec<u8>,
    periodo: Duration,
    janela_inicio: Option<Instant>,
    janela_ciclos: u32,
    /// A sensibilidade das placas já foi: vai uma vez, quando chega o primeiro pacote, e o barramento está vivo.
    placas_ajustadas: bool,
}

const JANELA: Duration = Duration::from_secs(1);

impl<C: Corpo> Laco<C> {
    pub fn new(corpo: C, p: &Params, intencoes: Arc<Intencoes>, saude: Arc<Saude>) -> Self {
        Self {
            seguranca: Seguranca::new(corpo, p),
            intencoes,
            saude,
            leitor: Leitor::new(),
            assinante: Assinante::new(p),
            pedidos: Vec::new(),
            idade_maxima_pacote: Duration::from_millis(p.telemetria.idade_maxima_ms),
            caixa: vec![0; 4096],
            periodo: Duration::from_secs_f64(1.0 / f64::from(p.laco.hz)),
            janela_inicio: None,
            janela_ciclos: 0,
            placas_ajustadas: false,
        }
    }

    /// Um ciclo. `quer_estado` é se há assinante: sem assinante nada é montado.
    pub fn ciclo(&mut self, agora: Instant, quer_estado: bool) -> Option<Estado> {
        match self.intencoes.tomar_energia() {
            Some(PedidoEnergia::Armar) => self.seguranca.armar(&self.saude),
            Some(PedidoEnergia::Desarmar) => self.seguranca.desarmar(&self.saude),
            None => {}
        }
        let leitura = self.intencoes.ler(agora);
        let levantado = self.levantado(agora);
        let aplicado = self
            .seguranca
            .aplicar(&leitura, levantado, agora, &self.saude);
        self.ler_corpo(agora);
        if !self.placas_ajustadas && self.saude.idade_do_pacote(agora).is_some() {
            self.seguranca.ajustar_placas(&self.saude);
            self.placas_ajustadas = true;
        }
        self.assinante.ciclo(agora, &mut self.pedidos);
        self.enviar_pedidos();
        if self.assinante.tomar_mudanca() {
            self.saude
                .publicar_amostra(Arc::new(self.assinante.amostra().clone()));
        }

        self.saude
            .armado
            .store(self.seguranca.armado(), Ordering::Relaxed);
        self.saude.carimbar_ciclo(agora);
        self.medir(agora);

        quer_estado.then(|| Estado {
            t: self.saude.segundos(),
            pedido: leitura.twist,
            aplicado: aplicado.twist,
            limitado_por: aplicado.limitado_por,
            torre: aplicado.torre,
            modo: leitura.modo,
            armado: self.seguranca.armado(),
            hz: self.saude.hz().unwrap_or(0.0),
            perdidos: self.saude.perdidos.load(Ordering::Relaxed),
            corpo: self.saude.amostra(),
            corpo_recente: self
                .saude
                .idade_do_pacote(agora)
                .is_some_and(|i| i < self.idade_maxima_pacote),
            idade_pacote_ms: self
                .saude
                .idade_do_pacote(agora)
                .map(|i| i.as_millis() as u64),
        })
    }

    /// A última coisa antes de sair: para o robô, como `robot.relax`. A gravação descarrega quando o corpo sai
    /// de escopo, ao fim da thread do laço.
    pub fn encerrar(&mut self) {
        self.seguranca.desarmar(&self.saude);
    }

    /// Quem chama avisa quando o trabalho passou do período.
    pub fn contar_trabalho(&self, duracao: Duration) {
        if duracao > self.periodo {
            self.saude.perdidos.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Levantado pelo nosso cálculo ou pelo bit do controlador, com telemetria recente: dado velho não diz onde o robô está.
    fn levantado(&self, agora: Instant) -> bool {
        let recente = self
            .saude
            .idade_do_pacote(agora)
            .is_some_and(|i| i < self.idade_maxima_pacote);
        let a = self.assinante.amostra();
        recente
            && (a.estado_chassi & BIT_LEVANTADO != 0
                || a.estimativa.as_ref().is_some_and(|c| c.levantado))
    }

    fn enviar_pedidos(&mut self) {
        for p in self.pedidos.drain(..) {
            self.seguranca.enviar_barramento(p, &self.saude);
        }
    }

    fn ler_corpo(&mut self, agora: Instant) {
        match self.seguranca.ler_corpo(&mut self.caixa) {
            Ok(0) => {}
            Ok(n) => {
                let quadros = self.leitor.empurrar(&self.caixa[..n]);
                self.saude
                    .quadros_recebidos
                    .fetch_add(quadros.len() as u64, Ordering::Relaxed);
                self.saude
                    .bytes_descartados
                    .store(self.leitor.descartados(), Ordering::Relaxed);
                let antes = self.assinante.amostra().pacotes;
                for q in &quadros {
                    self.assinante.receber(q, agora, &mut self.pedidos);
                }
                if self.assinante.amostra().pacotes != antes {
                    self.saude.carimbar_pacote(agora);
                }
            }
            Err(e) => tracing::debug!(erro = %e, "leitura do corpo falhou"),
        }
    }

    fn medir(&mut self, agora: Instant) {
        let inicio = *self.janela_inicio.get_or_insert(agora);
        self.janela_ciclos += 1;
        let passou = agora.saturating_duration_since(inicio);
        if passou >= JANELA {
            let ciclos = self.janela_ciclos.saturating_sub(1);
            self.saude
                .publicar_hz(f64::from(ciclos) / passou.as_secs_f64());
            self.janela_inicio = Some(agora);
            self.janela_ciclos = 1;
        }
    }
}
