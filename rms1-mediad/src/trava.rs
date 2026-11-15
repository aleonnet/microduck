//! A trava contra a própria voz: enquanto o robô toca um som, e por uma janela depois, o que o microfone ouve é descartado.
//!
//! No S1 o microfone ouve o alto-falante (o eco de 2026-10-31 tocou e gravou no mesmo aparelho). A regra é a do runtime antigo
//! do pato (`microduck_runtime/src/main.rs:2921-2949`): descartar enquanto toca e por 0,8 s depois. A janela é parâmetro
//! (`[midia] trava_ms`), porque a do S1 se mede no robô.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct Trava(Arc<Mutex<Estado>>);

struct Estado {
    janela: Duration,
    tocando: bool,
    /// Quando o último som terminou.
    terminou_em: Option<Instant>,
}

impl Trava {
    pub fn nova(janela: Duration) -> Self {
        Self(Arc::new(Mutex::new(Estado {
            janela,
            tocando: false,
            terminou_em: None,
        })))
    }

    /// O alto-falante começou a tocar.
    pub fn comecou(&self) {
        self.0.lock().expect("trava").tocando = true;
    }

    /// O alto-falante parou de tocar em `agora`.
    pub fn terminou(&self, agora: Instant) {
        let mut e = self.0.lock().expect("trava");
        e.tocando = false;
        e.terminou_em = Some(agora);
    }

    /// Se o que o microfone ouve em `agora` deve ser descartado.
    pub fn vale(&self, agora: Instant) -> bool {
        let e = self.0.lock().expect("trava");
        e.tocando
            || e.terminou_em
                .is_some_and(|fim| agora.saturating_duration_since(fim) < e.janela)
    }
}
