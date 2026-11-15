//! O microfone: as fontes, o vigia de som do pato e a trava contra a própria voz.
//!
//! O vigia é o do `pet-detect` (`pet_detect::worker::SoundSentry`): um envelope de RMS com piso que se adapta, que diz
//! `Noise` para um estalo e `Voice` para uma fala. Aqui não há classificador de carinho: o do pato não serve ao S1.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

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

/// O nível de um bloco, de 0 a 1: a raiz da média dos quadrados. É o que `s1.media.status` mostra em `mic_level`.
pub fn nivel(amostras: &[f32]) -> f32 {
    if amostras.is_empty() {
        return 0.0;
    }
    (amostras.iter().map(|a| a * a).sum::<f32>() / amostras.len() as f32).sqrt()
}

/// O que o microfone do robô conta a quem serve: o que ouviu, o nível de cada bloco e por que não está ouvindo.
pub struct Saidas {
    pub ouvido: Box<dyn Fn(Ouvido) + Send>,
    /// O nível do último bloco, como os bits de um `f32`.
    pub nivel: Arc<AtomicU32>,
    pub dizer: Box<dyn Fn(Option<String>) + Send>,
}

/// O microfone do robô, até `parar` ficar verdadeiro: um programa que escreve na saída PCM cru, S16LE, mono, a 16 kHz
/// (no robô, `arecord -D micleft -f S16_LE -r 16000 -c 1 -t raw`, como o trabalhador do pato). A captura é de um cliente
/// só, e por isso é um processo só, daqui. Se ele cai, sobe de novo depois de `espera`, e o motivo vai a `dizer`.
pub fn rodar_comando(
    programa: &str,
    args: &[String],
    trava: Trava,
    saidas: Saidas,
    espera: Duration,
    parar: Arc<AtomicBool>,
) {
    let mut ouvinte = Ouvinte::novo(trava);
    let mut bytes = vec![0u8; BLOCO * 2];
    let mut amostras = vec![0f32; BLOCO];
    while !parar.load(Ordering::Relaxed) {
        let filho = Command::new(programa)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut filho = match filho {
            Ok(f) => f,
            Err(e) => {
                (saidas.dizer)(Some(format!("o microfone não abriu ({programa}): {e}")));
                std::thread::sleep(espera);
                continue;
            }
        };
        let mut saida = filho
            .stdout
            .take()
            .expect("a saída do programa do microfone");
        let mut blocos = 0u64;
        while !parar.load(Ordering::Relaxed) && saida.read_exact(&mut bytes).is_ok() {
            for (a, b) in amostras.iter_mut().zip(bytes.as_chunks::<2>().0) {
                *a = f32::from(i16::from_le_bytes(*b)) / 32768.0;
            }
            if blocos == 0 {
                (saidas.dizer)(None);
            }
            blocos += 1;
            saidas
                .nivel
                .store(nivel(&amostras).to_bits(), Ordering::Relaxed);
            for o in ouvinte.empurrar(&amostras, Instant::now()) {
                (saidas.ouvido)(o);
            }
        }
        let _ = filho.kill();
        let _ = filho.wait();
        if !parar.load(Ordering::Relaxed) {
            saidas.nivel.store(0f32.to_bits(), Ordering::Relaxed);
            (saidas.dizer)(Some(format!(
                "o microfone parou depois de {blocos} blocos ({programa}); tentando de novo"
            )));
            std::thread::sleep(espera);
        }
    }
}
