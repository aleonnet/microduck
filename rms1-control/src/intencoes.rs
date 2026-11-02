//! O que os clientes pedem. Escrito pelo socket, lido uma vez por ciclo pelo laço. Cada campo é uma
//! troca atômica: nenhum dos lados espera o outro.
//!
//! Giro do corpo e torre são campos separados, com carimbo, porque a pergunta do laço nunca é "qual é o
//! valor", e sim "de quando é". É isso que o prazo lê.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use arc_swap::{ArcSwap, ArcSwapOption};

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

pub struct Intencoes {
    twist: ArcSwapOption<Carimbado<[f64; 3]>>,
    /// Yaw e pitch do contrato, em radianos. `None` até alguém pedir: armar não leva a torre a lugar nenhum.
    torre: ArcSwap<Option<[f64; 2]>>,
    energia: AtomicU8,
    modo: AtomicU8,
    armado_pedido: AtomicBool,
}

/// O que o laço lê no começo do ciclo.
#[derive(Debug, Clone, Copy)]
pub struct Leitura {
    pub twist: [f64; 3],
    /// `None` se ninguém nunca pediu movimento.
    pub idade_twist: Option<Duration>,
    pub torre: Option<[f64; 2]>,
    pub modo: Modo,
}

impl Default for Intencoes {
    fn default() -> Self {
        Self {
            twist: ArcSwapOption::empty(),
            torre: ArcSwap::from_pointee(None),
            energia: AtomicU8::new(ENERGIA_NADA),
            modo: AtomicU8::new(Modo::Livre.codigo()),
            armado_pedido: AtomicBool::new(false),
        }
    }
}

impl Intencoes {
    pub fn pedir_twist(&self, twist: [f64; 3]) {
        self.twist.store(Some(Arc::new(Carimbado {
            valor: twist,
            em: Instant::now(),
        })));
    }

    pub fn pedir_torre(&self, yaw: f64, pitch: f64) {
        self.torre.store(Arc::new(Some([yaw, pitch])));
    }

    pub fn parar(&self) {
        self.pedir_twist([0.0; 3]);
    }

    pub fn pedir_modo(&self, modo: Modo) {
        self.modo.store(modo.codigo(), Ordering::Relaxed);
    }

    pub fn modo(&self) -> Modo {
        Modo::de_codigo(self.modo.load(Ordering::Relaxed))
    }

    pub fn pedir_armar(&self, sim: bool) {
        self.armado_pedido.store(sim, Ordering::Relaxed);
        self.energia.store(
            if sim { ENERGIA_ARMAR } else { ENERGIA_DESARMAR },
            Ordering::Relaxed,
        );
    }

    /// O que o último cliente pediu, para a alternância de `robot.enable`.
    pub fn armado_pedido(&self) -> bool {
        self.armado_pedido.load(Ordering::Relaxed)
    }

    pub fn tomar_energia(&self) -> Option<PedidoEnergia> {
        match self.energia.swap(ENERGIA_NADA, Ordering::Relaxed) {
            ENERGIA_ARMAR => Some(PedidoEnergia::Armar),
            ENERGIA_DESARMAR => Some(PedidoEnergia::Desarmar),
            _ => None,
        }
    }

    pub fn ler(&self, agora: Instant) -> Leitura {
        let t = self.twist.load_full();
        Leitura {
            twist: t.as_ref().map_or([0.0; 3], |c| c.valor),
            idade_twist: t.as_ref().map(|c| agora.saturating_duration_since(c.em)),
            torre: **self.torre.load(),
            modo: self.modo(),
        }
    }
}
