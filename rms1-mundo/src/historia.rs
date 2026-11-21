//! A torre e a pose do último segundo, para casar cada quadro com o instante em que foi tirado.
//!
//! É o molde de `History`, do cérebro do pato (`autod/src/obstacle.rs`): a amostra de agora está até um quadro inteiro fora
//! do instante da captura, e com a torre girando isso é muito giro.

use std::collections::VecDeque;

/// O robô num instante: de onde a câmera olhava.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Amostra {
    /// Segundos, no relógio de quem chama. O mesmo relógio da hora de captura das grades.
    pub t: f64,
    /// A torre, em radianos: o giro em relação ao chassi, positivo à esquerda, e a inclinação em relação ao chão, positiva
    /// para cima.
    pub torre: [f64; 2],
    /// A odometria: x e y em metros, e o rumo em radianos, positivo à esquerda.
    pub pose: [f64; 3],
}

/// As amostras do último segundo.
#[derive(Debug, Default)]
pub struct Historia(VecDeque<Amostra>);

/// Quanto tempo de amostras fica guardado.
const GUARDA_S: f64 = 1.0;

impl Historia {
    /// Guarda a amostra e solta as que ficaram mais velhas que um segundo. A que chega com a hora de antes da última (o
    /// relógio de quem chama recomeçou) esvazia a história: amostras de dois relógios não se comparam.
    pub fn guardar(&mut self, a: Amostra) {
        if self.0.back().is_some_and(|u| a.t < u.t) {
            self.0.clear();
        }
        self.0.push_back(a);
        while self.0.front().is_some_and(|p| a.t - p.t > GUARDA_S) {
            self.0.pop_front();
        }
    }

    /// A amostra mais perto do instante `t`.
    pub fn mais_perto(&self, t: f64) -> Option<Amostra> {
        self.0
            .iter()
            .min_by(|a, b| (a.t - t).abs().total_cmp(&(b.t - t).abs()))
            .copied()
    }

    pub fn ultima(&self) -> Option<Amostra> {
        self.0.back().copied()
    }
}
