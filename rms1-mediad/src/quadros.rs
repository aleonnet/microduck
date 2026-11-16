//! O encontro de quadros: quem quer um quadro pede, e recebe o próximo que for capturado.
//!
//! Refeito de `mediad/src/pipeline.rs:283-387`, sem o GStreamer. As três regras são as do pato:
//!
//! - **Pedido, e não publicado.** Copiar um quadro custa o quadro inteiro (1.382.400 bytes no S1). Quadro que ninguém pediu
//!   custa uma leitura atômica, e mais nada.
//! - **O próximo, e não o último.** O pedido é respondido por uma captura feita depois dele. Quem recebesse o último quadro
//!   guardado veria uma cena de antes do pedido.
//! - **Uma captura responde a todos os que esperam.** Dois pedidos ao mesmo tempo custam uma cópia.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

/// Um quadro como saiu da fonte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quadro {
    pub largura: u32,
    pub altura: u32,
    /// O nome do formato, levado junto do quadro: quem lê não precisa supor.
    pub formato: &'static str,
    /// Quando foi capturado. É hora de observação, e nunca relógio de ritmo.
    pub capturado_em: SystemTime,
    pub dados: Vec<u8>,
}

/// Quanto quem pediu espera pelo quadro. O do pato (`FRAME_TIMEOUT`): quinze períodos a 30 quadros por segundo.
pub const PRAZO: Duration = Duration::from_millis(500);

#[derive(Clone, Default)]
pub struct Quadros(Arc<Partilhado>);

#[derive(Default)]
struct Partilhado {
    ultimo: Mutex<Ultimo>,
    entregue: Condvar,
    pedido: AtomicBool,
    /// Quantos quadros foram copiados para entrega: é o que os testes e `s1.media.status` leem.
    copias: AtomicU64,
}

#[derive(Default)]
struct Ultimo {
    quadro: Option<Arc<Quadro>>,
    geracao: u64,
}

impl Quadros {
    /// Pede o próximo quadro e espera por ele até [`PRAZO`]. `None` se nenhum chegou: nunca o quadro de antes do pedido.
    pub fn proximo(&self) -> Option<Arc<Quadro>> {
        self.proximo_em(PRAZO)
    }

    pub fn proximo_em(&self, prazo: Duration) -> Option<Arc<Quadro>> {
        // A geração é lida antes de marcar o pedido: uma captura que chega entre os dois foi pedida por este leitor, até
        // onde ele consegue saber.
        let ultimo = self.0.ultimo.lock().expect("trava dos quadros");
        let vista = ultimo.geracao;
        self.0.pedido.store(true, Ordering::Relaxed);
        let (ultimo, _) = self
            .0
            .entregue
            .wait_timeout_while(ultimo, prazo, |u| u.geracao == vista)
            .expect("trava dos quadros");
        // No prazo vencido o que está guardado é de antes do pedido: dito como o prazo vencido que é, e nunca entregue.
        if ultimo.geracao == vista {
            return None;
        }
        ultimo.quadro.clone()
    }

    /// Se alguém espera um quadro, e leva o pedido. É tudo o que a fonte faz num quadro que ninguém pediu.
    pub fn levar_pedido(&self) -> bool {
        self.0.pedido.swap(false, Ordering::Relaxed)
    }

    /// Entrega a captura a todos os que esperam: uma cópia, por mais que sejam.
    pub fn entregar(&self, quadro: Quadro) {
        drop(self.trocar(quadro));
    }

    /// Entrega o quadro e devolve o que estava guardado antes dele. A fonte que copia um quadro grande a cada entrega
    /// reaproveita a memória do anterior, se ninguém mais o segura ([`Self::memoria_de`]).
    pub fn trocar(&self, quadro: Quadro) -> Option<Arc<Quadro>> {
        let mut ultimo = self.0.ultimo.lock().expect("trava dos quadros");
        let anterior = ultimo.quadro.replace(Arc::new(quadro));
        ultimo.geracao = ultimo.geracao.wrapping_add(1);
        drop(ultimo);
        self.0.copias.fetch_add(1, Ordering::Relaxed);
        self.0.entregue.notify_all();
        anterior
    }

    /// A memória de um quadro que já saiu de cena, para ser escrita de novo: `None` se alguém ainda o lê.
    pub fn memoria_de(anterior: Option<Arc<Quadro>>) -> Option<Vec<u8>> {
        Arc::try_unwrap(anterior?).ok().map(|q| q.dados)
    }

    pub fn copias(&self) -> u64 {
        self.0.copias.load(Ordering::Relaxed)
    }
}
