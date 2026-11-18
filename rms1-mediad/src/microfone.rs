//! O microfone: as fontes, o vigia de som do pato e a trava contra a própria voz.
//!
//! O vigia é o do `pet-detect` (`pet_detect::worker::SoundSentry`): um envelope de RMS com piso que se adapta, que diz
//! `Noise` para um estalo e `Voice` para uma fala. Aqui não há classificador de carinho: o do pato não serve ao S1.
//!
//! **O trecho de voz** que vai à estação é cortado aqui, ao lado do vigia, e não por ele: o vigia só avisa da voz quando ela
//! termina, e trata o som de mais de 3 s como ruído contínuo, sem avisar (`pet-detect/src/worker.rs`). O guardador mede o
//! nível a cada 32 ms contra um piso que acompanha a sala: a fala começa quando o nível passa de um múltiplo do piso, leva
//! junto o som de pouco antes, e termina no silêncio ou no teto. Os números são os de `[midia.voz]`.

use std::collections::VecDeque;
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

/// Como cortar do microfone o trecho de voz: os números de `[midia.voz]`.
#[derive(Debug, Clone, Copy)]
pub struct Voz {
    /// Quanto do som de antes do começo da fala vai junto.
    pub folga: Duration,
    /// Quanto de silêncio encerra o trecho.
    pub silencio: Duration,
    /// O maior trecho.
    pub teto: Duration,
    /// O menor som alto que conta como fala.
    pub minimo: Duration,
    /// Quantas vezes o piso o nível tem de passar para a fala começar, e abaixo de quantas conta como silêncio.
    pub entra: f32,
    pub sai: f32,
}

/// De quanto em quanto o guardador mede o nível: 32 ms a 16 kHz, o quadro do vigia.
const QUADRO: usize = 512;

fn amostras_em(d: Duration) -> usize {
    (d.as_millis() as usize) * 16
}

/// Guarda o trecho de voz em curso. Fora da fala, guarda só o som de pouco antes.
struct Guardador {
    cfg: Voz,
    /// O nível da sala fora da fala. Começa no do vigia.
    piso: f32,
    /// As amostras que ainda não fecham um quadro.
    resto: Vec<i16>,
    /// O som de antes: as últimas amostras fora da fala, até a folga.
    antes: VecDeque<i16>,
    /// A fala em curso.
    trecho: Option<Vec<i16>>,
    /// No trecho em curso: quantos quadros foram de som alto, e quantos de silêncio vieram em seguida.
    altos: usize,
    quietos: usize,
    /// O trecho que terminou, à espera de quem o leve.
    pronto: Option<Vec<i16>>,
    /// Depois de ficar surdo, o guardador só volta a armar quando ouve a sala quieta por um tempo: a fala de alguém que já
    /// ia pelo meio quando a trava abriu não é uma frase. Aqui, quantos quadros quietos seguidos já ouviu; `None` é armado.
    quieto_antes: Option<usize>,
}

/// Quanto de sala quieta o guardador ouve, depois de surdo, antes de voltar a armar.
const QUIETO_PARA_ARMAR: Duration = Duration::from_millis(300);

impl Guardador {
    fn novo(cfg: Voz) -> Self {
        Self {
            cfg,
            piso: 0.003,
            resto: Vec::with_capacity(QUADRO),
            antes: VecDeque::new(),
            trecho: None,
            altos: 0,
            quietos: 0,
            pronto: None,
            quieto_antes: None,
        }
    }

    /// Recebe um bloco. Com `surdo` (o robô está tocando som, ou acabou de tocar), o que está em curso é largado: seria a
    /// voz do próprio robô, ou uma fala coberta por ela.
    fn empurrar(&mut self, amostras: &[f32], surdo: bool) {
        if surdo {
            self.resto.clear();
            self.antes.clear();
            self.trecho = None;
            self.quieto_antes = Some(0);
            return;
        }
        for a in amostras {
            self.resto
                .push((a * 32768.0).clamp(-32768.0, 32767.0) as i16);
            if self.resto.len() == QUADRO {
                let quadro = std::mem::replace(&mut self.resto, Vec::with_capacity(QUADRO));
                self.quadro(&quadro);
            }
        }
    }

    fn quadro(&mut self, quadro: &[i16]) {
        let nivel = (quadro
            .iter()
            .map(|a| {
                let x = f32::from(*a) / 32768.0;
                x * x
            })
            .sum::<f32>()
            / quadro.len() as f32)
            .sqrt();
        // Os mínimos absolutos são os do vigia: sem eles, um múltiplo de um piso quase zero dispararia no silêncio.
        let entra = (self.piso * self.cfg.entra).max(0.002);
        let sai = (self.piso * self.cfg.sai).max(0.0012);
        // Saindo de surdo: espera a sala quieta. O som de antes segue sendo guardado, para a frase seguinte sair com a folga.
        if let Some(quietos) = self.quieto_antes {
            let quietos = if nivel < sai { quietos + 1 } else { 0 };
            self.quieto_antes = (quietos * QUADRO < amostras_em(QUIETO_PARA_ARMAR)).then_some(quietos);
            self.antes.extend(quadro);
            let sobra = self.antes.len().saturating_sub(amostras_em(self.cfg.folga));
            self.antes.drain(..sobra);
            return;
        }
        let Some(trecho) = self.trecho.as_mut() else {
            if nivel > entra {
                let mut novo: Vec<i16> = self.antes.drain(..).collect();
                novo.extend_from_slice(quadro);
                self.trecho = Some(novo);
                (self.altos, self.quietos) = (1, 0);
            } else {
                // O piso só acompanha a sala fora da fala, no passo do vigia (cerca de 6 s).
                self.piso = 0.995 * self.piso + 0.005 * nivel;
                self.antes.extend(quadro);
                let sobra = self.antes.len().saturating_sub(amostras_em(self.cfg.folga));
                self.antes.drain(..sobra);
            }
            return;
        };
        trecho.extend_from_slice(quadro);
        if nivel < sai {
            self.quietos += 1;
        } else {
            self.quietos = 0;
            self.altos += 1;
        }
        let teto = amostras_em(self.cfg.teto);
        let no_teto = trecho.len() >= teto;
        if !no_teto && self.quietos * QUADRO < amostras_em(self.cfg.silencio) {
            return;
        }
        let mut fala = self.trecho.take().expect("o trecho em curso");
        fala.truncate(teto);
        if no_teto {
            // O som que não para deixa de contar como fala: o piso sobe até ele, como o vigia faz com o ruído contínuo.
            self.piso = self.piso.max(nivel * 0.5);
        }
        // O som alto curto é estalo ou palma, e não vai.
        if self.altos * QUADRO >= amostras_em(self.cfg.minimo) {
            self.pronto = Some(fala);
        }
    }
}

/// O ouvido do robô: o vigia sobre as amostras de qualquer fonte, com a trava; e, se pedido, o guardador do trecho de voz.
pub struct Ouvinte {
    vigia: SoundSentry,
    trava: Trava,
    voz: Option<Guardador>,
}

impl Ouvinte {
    pub fn novo(trava: Trava) -> Self {
        Self {
            vigia: SoundSentry::new(),
            trava,
            voz: None,
        }
    }

    /// O ouvinte que também guarda o trecho de voz, para a estação.
    pub fn com_voz(trava: Trava, voz: Voz) -> Self {
        Self {
            voz: Some(Guardador::novo(voz)),
            ..Self::novo(trava)
        }
    }

    /// O trecho de voz que terminou no último bloco, se terminou um: PCM de 16 bits, 16 kHz, um canal.
    pub fn trecho(&mut self) -> Option<Vec<i16>> {
        self.voz.as_mut().and_then(|g| g.pronto.take())
    }

    /// Entrega ao vigia um bloco de amostras a 16 kHz, mono, de -1 a 1, ouvido em `agora`, e devolve o que saiu.
    pub fn empurrar(&mut self, amostras: &[f32], agora: Instant) -> Vec<Ouvido> {
        // O vigia ouve sempre, para o piso dele acompanhar a sala; o que a trava descarta é o evento.
        let mut eventos = Vec::new();
        self.vigia.push(amostras, false, &mut eventos);
        let surdo = self.trava.vale(agora);
        if let Some(guardador) = self.voz.as_mut() {
            guardador.empurrar(amostras, surdo);
        }
        if surdo {
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
    /// Cada trecho de voz que terminou, se o ouvinte guarda voz.
    pub trecho: Box<dyn Fn(Vec<i16>) + Send>,
}

/// Passa pelo ouvinte os blocos lidos de `saida`, até ela acabar ou `parar` valer. Devolve quantos blocos ouviu. Uma fonte
/// calada por mais que o prazo de leitura dela não é fim: o nível vai a zero e a espera segue.
fn ouvir_fluxo(
    saida: &mut impl Read,
    ouvinte: &mut Ouvinte,
    saidas: &Saidas,
    parar: &AtomicBool,
) -> u64 {
    let mut bytes = vec![0u8; BLOCO * 2];
    let mut amostras = vec![0f32; BLOCO];
    let mut blocos = 0u64;
    let mut cheio = 0usize;
    while !parar.load(Ordering::Relaxed) {
        match saida.read(&mut bytes[cheio..]) {
            Ok(0) => break,
            Ok(n) => cheio += n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                saidas.nivel.store(0f32.to_bits(), Ordering::Relaxed);
                continue;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
        if cheio < bytes.len() {
            continue;
        }
        cheio = 0;
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
        if let Some(trecho) = ouvinte.trecho() {
            (saidas.trecho)(trecho);
        }
    }
    blocos
}

/// O microfone do robô, até `parar` ficar verdadeiro: um programa que escreve na saída PCM cru, S16LE, mono, a 16 kHz
/// (no robô, `arecord -D micleft -f S16_LE -r 16000 -c 1 -t raw`, como o trabalhador do pato). A captura é de um cliente
/// só, e por isso é um processo só, daqui. Se ele cai, sobe de novo depois de `espera`, e o motivo vai a `dizer`.
pub fn rodar_comando(
    programa: &str,
    args: &[String],
    mut ouvinte: Ouvinte,
    saidas: Saidas,
    espera: Duration,
    parar: Arc<AtomicBool>,
) {
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
        let blocos = ouvir_fluxo(&mut saida, &mut ouvinte, &saidas, &parar);
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

/// A primeira coisa que o microfone simulado manda ao serviço local: quem liga quer ouvir.
pub const OUVE: u8 = b'O';
/// Quanto o microfone simulado espera por som antes de dizer que a sala está calada.
const SILENCIO: Duration = Duration::from_millis(400);

/// O microfone do robô simulado, até `parar` ficar verdadeiro: o serviço local do simulador manda o som da página (o que
/// ela mesma toca e, se o dono ligar, o microfone do Mac) em PCM cru, S16LE, mono, a 16 kHz. Sem o serviço, tenta de novo
/// depois de `espera`, e o motivo vai a `dizer`. Página calada não é defeito: o nível fica em zero.
pub fn rodar_simulador(
    endereco: &str,
    mut ouvinte: Ouvinte,
    saidas: Saidas,
    espera: Duration,
    parar: Arc<AtomicBool>,
) {
    use std::io::Write;
    while !parar.load(Ordering::Relaxed) {
        let ligada = std::net::TcpStream::connect(endereco).and_then(|mut c| {
            c.set_read_timeout(Some(SILENCIO))?;
            c.write_all(&[OUVE])?;
            Ok(c)
        });
        let mut conexao = match ligada {
            Ok(c) => c,
            Err(e) => {
                (saidas.dizer)(Some(format!(
                    "sem o microfone do simulador em {endereco}: {e}"
                )));
                std::thread::sleep(espera);
                continue;
            }
        };
        (saidas.dizer)(None);
        ouvir_fluxo(&mut conexao, &mut ouvinte, &saidas, &parar);
        saidas.nivel.store(0f32.to_bits(), Ordering::Relaxed);
        if !parar.load(Ordering::Relaxed) {
            (saidas.dizer)(Some(format!(
                "o microfone do simulador em {endereco} caiu; tentando de novo"
            )));
            std::thread::sleep(espera);
        }
    }
}
