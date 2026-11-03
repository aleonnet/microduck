//! O que o laço publica a cada ciclo. O socket calcula a saúde daqui, sem perguntar nada ao laço: é isso
//! que deixa um laço travado se denunciar em vez de pendurar quem pergunta.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;

use crate::telemetria::Amostra;

pub struct Saude {
    inicio: Instant,
    pub ciclos: AtomicU64,
    pub perdidos: AtomicU64,
    ultimo_us: AtomicU64,
    /// mHz, zero até fechar a primeira janela.
    hz_milli: AtomicU64,
    pub erros_seguidos: AtomicU32,
    pub armado: AtomicBool,
    pub bytes_descartados: AtomicU64,
    pub quadros_recebidos: AtomicU64,
    pub erros_barramento: AtomicU64,
    /// O retrato do corpo, publicado pelo laço. Quem pergunta nunca espera por ele.
    amostra: ArcSwapOption<Amostra>,
    /// Microssegundos desde `inicio`, mais um; zero é nunca.
    ultimo_pacote_us: AtomicU64,
}

impl Default for Saude {
    fn default() -> Self {
        Self {
            inicio: Instant::now(),
            ciclos: AtomicU64::new(0),
            perdidos: AtomicU64::new(0),
            ultimo_us: AtomicU64::new(0),
            hz_milli: AtomicU64::new(0),
            erros_seguidos: AtomicU32::new(0),
            armado: AtomicBool::new(false),
            bytes_descartados: AtomicU64::new(0),
            quadros_recebidos: AtomicU64::new(0),
            erros_barramento: AtomicU64::new(0),
            amostra: ArcSwapOption::empty(),
            ultimo_pacote_us: AtomicU64::new(0),
        }
    }
}

/// O veredito e os números de onde saiu.
#[derive(Debug, Clone, PartialEq)]
pub struct Veredito {
    pub saudavel: bool,
    /// O motivo é o corpo, e não este programa.
    pub degradado: bool,
    pub motivo: Option<String>,
    pub hz: Option<f64>,
    pub ciclos: u64,
    pub perdidos: u64,
    pub idade_ultimo: Duration,
    pub erros_seguidos: u32,
}

impl Saude {
    pub fn segundos(&self) -> f64 {
        self.inicio.elapsed().as_secs_f64()
    }

    pub fn carimbar_ciclo(&self, agora: Instant) {
        self.ultimo_us.store(
            agora.saturating_duration_since(self.inicio).as_micros() as u64,
            Ordering::Relaxed,
        );
        self.ciclos.fetch_add(1, Ordering::Relaxed);
    }

    pub fn publicar_amostra(&self, a: Arc<Amostra>) {
        self.amostra.store(Some(a));
    }

    pub fn amostra(&self) -> Option<Arc<Amostra>> {
        self.amostra.load_full()
    }

    pub fn carimbar_pacote(&self, agora: Instant) {
        let us = agora.saturating_duration_since(self.inicio).as_micros() as u64;
        self.ultimo_pacote_us.store(us + 1, Ordering::Relaxed);
    }

    /// `None` se nenhum pacote chegou.
    pub fn idade_do_pacote(&self, agora: Instant) -> Option<Duration> {
        match self.ultimo_pacote_us.load(Ordering::Relaxed) {
            0 => None,
            us => Some(
                agora
                    .saturating_duration_since(self.inicio)
                    .saturating_sub(Duration::from_micros(us - 1)),
            ),
        }
    }

    pub fn publicar_hz(&self, hz: f64) {
        self.hz_milli.store((hz * 1000.0) as u64, Ordering::Relaxed);
    }

    pub fn hz(&self) -> Option<f64> {
        match self.hz_milli.load(Ordering::Relaxed) {
            0 => None,
            m => Some(m as f64 / 1000.0),
        }
    }

    pub fn veredito(&self, agora: Instant, hz_minimo: f64, idade_maxima: Duration) -> Veredito {
        let ciclos = self.ciclos.load(Ordering::Relaxed);
        let idade_ultimo =
            agora
                .saturating_duration_since(self.inicio)
                .saturating_sub(Duration::from_micros(
                    self.ultimo_us.load(Ordering::Relaxed),
                ));
        let hz = self.hz();
        let erros_seguidos = self.erros_seguidos.load(Ordering::Relaxed);
        let (saudavel, degradado, motivo) = if ciclos == 0 && idade_ultimo > idade_maxima {
            (
                false,
                false,
                Some("o laço não completou nenhum ciclo".to_owned()),
            )
        } else if ciclos > 0 && idade_ultimo > idade_maxima {
            (
                false,
                false,
                Some(format!("o laço parou há {} ms", idade_ultimo.as_millis())),
            )
        } else if hz.is_some_and(|h| h < hz_minimo) {
            (
                false,
                false,
                Some(format!("laço a {:.1} Hz", hz.unwrap_or(0.0))),
            )
        } else if erros_seguidos > 0 {
            (
                false,
                true,
                Some(format!(
                    "o corpo não recebe: {erros_seguidos} envios falharam em seguida"
                )),
            )
        } else {
            (true, false, None)
        };
        Veredito {
            saudavel,
            degradado,
            motivo,
            hz,
            ciclos,
            perdidos: self.perdidos.load(Ordering::Relaxed),
            idade_ultimo,
            erros_seguidos,
        }
    }
}
