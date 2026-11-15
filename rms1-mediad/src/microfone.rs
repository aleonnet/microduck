//! O microfone: as fontes, o vigia de som do pato e a trava contra a própria voz.
//!
//! O vigia é o do `pet-detect` (`pet_detect::worker::SoundSentry`): um envelope de RMS com piso que se adapta, que diz
//! `Noise` para um estalo e `Voice` para uma fala. Aqui não há classificador de carinho: o do pato não serve ao S1.

use std::time::Instant;

use pet_detect::worker::{SoundEvent, SoundSentry};

use crate::trava::Trava;

/// O que o robô ouviu, já com a trava aplicada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ouvido {
    Barulho,
    Voz,
}

impl Ouvido {
    /// O nome no fio, em `s1.audio.heard {"kind"}`: os do vigia do pato.
    pub fn nome(self) -> &'static str {
        match self {
            Ouvido::Barulho => "noise",
            Ouvido::Voz => "voice",
        }
    }
}

/// O ouvido do robô: o vigia sobre as amostras de qualquer fonte, com a trava.
pub struct Ouvinte {
    vigia: SoundSentry,
    trava: Trava,
}

impl Ouvinte {
    pub fn novo(trava: Trava) -> Self {
        Self {
            vigia: SoundSentry::new(),
            trava,
        }
    }

    /// Entrega ao vigia um bloco de amostras a 16 kHz, mono, de -1 a 1, ouvido em `agora`, e devolve o que saiu.
    pub fn empurrar(&mut self, amostras: &[f32], agora: Instant) -> Vec<Ouvido> {
        // O vigia ouve sempre, para o piso dele acompanhar a sala; o que a trava descarta é o evento.
        let mut eventos = Vec::new();
        self.vigia.push(amostras, false, &mut eventos);
        if self.trava.vale(agora) {
            return Vec::new();
        }
        eventos
            .into_iter()
            .map(|e| match e {
                SoundEvent::Noise => Ouvido::Barulho,
                SoundEvent::Voice => Ouvido::Voz,
            })
            .collect()
    }
}

/// O tamanho do bloco entregue ao vigia: 128 ms a 16 kHz, o que o `arecord` do pato entrega por leitura.
pub const BLOCO: usize = 2048;

/// A fonte de arquivo: um `.wav` qualquer, levado a 16 kHz mono como o pato faz (`pet_detect::load_wav_mono_16k`).
pub fn ler_arquivo(caminho: &str) -> anyhow::Result<Vec<f32>> {
    pet_detect::load_wav_mono_16k(caminho)
}
