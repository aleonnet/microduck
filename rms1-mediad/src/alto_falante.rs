//! O alto-falante: o tocador do pato, derivado de `robotd/src/sound.rs` (que mora num binário, e por isso não se importa).
//!
//! As regras são as de lá, porque a saída de som é de um cliente só:
//!
//! - **Um som por vez, e o novo corta o anterior.**
//! - **O `wheee` vai num fluxo só:** começo, laço enquanto o pedido se renova, e fim. Ele tem duas saídas, e não são a mesma:
//!   quem diz `hold: false` corta onde está; o pedido que só deixou de chegar por 300 ms (`robotd/src/intents.rs:119`) pousa,
//!   tocando o fim. Durante o `wheee`, um som avulso é descartado, e não enfileirado.
//!
//! O que não veio: o teremim e o coral, que o S1 não tem. O que é daqui: a saída é uma interface ([`Saida`]), para o robô
//! (`aplay`), o simulador e o teste tocarem pelo mesmo tocador; e o tocador avisa a [`Trava`] quando começa e quando para.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::trava::Trava;

/// Por quanto tempo um pedido de `wheee` vale sem ser renovado: o `WHEEE_HOLD_FRESH` do pato.
pub const WHEEE_VALE: Duration = Duration::from_millis(300);

/// Um som que está tocando.
pub trait Tocando: Send {
    /// Se já terminou sozinho.
    fn terminou(&mut self) -> bool;
    /// Para agora.
    fn cortar(&mut self);
}

/// Por onde o som sai.
pub trait Saida: Send {
    /// Toca um `.wav` inteiro.
    fn tocar_arquivo(&mut self, wav: &Path) -> std::io::Result<Box<dyn Tocando>>;
    /// Abre um fluxo de PCM cru (S16LE, mono, `hz`), para o `wheee`: quem escreve, e o som que está tocando.
    fn abrir_fluxo(
        &mut self,
        hz: u32,
    ) -> std::io::Result<(Box<dyn Write + Send>, Box<dyn Tocando>)>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Passeio {
    Fora,
    Andando,
    Pousando,
}

pub struct Tocador {
    banco: PathBuf,
    saida: Box<dyn Saida>,
    trava: Trava,
    atual: Option<Box<dyn Tocando>>,
    passeio: Passeio,
    /// O laço do `wheee` segue enquanto isto vale. Apagar sem cortar o som é o que faz o fim tocar.
    seguro: Arc<AtomicBool>,
    renovado_em: Option<Instant>,
    avisou_banco: bool,
}

impl Tocador {
    pub fn novo(banco: PathBuf, saida: Box<dyn Saida>, trava: Trava) -> Self {
        Self {
            banco,
            saida,
            trava,
            atual: None,
            passeio: Passeio::Fora,
            seguro: Arc::new(AtomicBool::new(false)),
            renovado_em: None,
            avisou_banco: false,
        }
    }

    /// Toca um som do banco, cortando o que estiver tocando. Durante o `wheee`, é descartado: é um acontecimento, e um
    /// acontecimento que chega no meio do passeio já passou quando o passeio termina.
    pub fn tocar(&mut self, tag: &str, agora: Instant) {
        if self.passeio != Passeio::Fora {
            tracing::debug!(tag, "som descartado: o wheee está com a saída");
            return;
        }
        self.cortar(agora);
        let Some(wav) = escolher(&self.banco.join(tag)) else {
            self.avisar_banco();
            return;
        };
        match self.saida.tocar_arquivo(&wav) {
            Ok(som) => {
                self.atual = Some(som);
                self.trava.comecou();
            }
            Err(e) => tracing::debug!(erro = %e, tag, "a saída de som recusou"),
        }
    }

    /// O `wheee`: `true` começa ou renova o pedido; `false` corta onde está.
    pub fn wheee(&mut self, segurar: bool, agora: Instant) {
        if !segurar {
            if self.passeio != Passeio::Fora {
                self.cortar(agora);
            }
            return;
        }
        // Um pedido durante o pouso recomeça o passeio.
        if self.passeio != Passeio::Andando {
            self.comecar_wheee(agora);
        }
        self.renovado_em = Some(agora);
    }

    /// A cada tique: pousa o `wheee` que deixou de ser renovado, recolhe o som que terminou e avisa a trava.
    pub fn tique(&mut self, agora: Instant) {
        let vencido = self
            .renovado_em
            .is_none_or(|t| agora.saturating_duration_since(t) >= WHEEE_VALE);
        if self.passeio == Passeio::Andando && vencido {
            // Quem escreve sai do laço com o fluxo aberto, e toca o fim: ninguém pediu um corte.
            self.seguro.store(false, Ordering::Relaxed);
            self.passeio = Passeio::Pousando;
        }
        if self.atual.as_mut().is_some_and(|som| som.terminou()) {
            self.atual = None;
            self.passeio = Passeio::Fora;
            self.trava.terminou(agora);
        }
    }

    /// O serviço vai sair: nenhum som fica tocando.
    pub fn parar(&mut self, agora: Instant) {
        self.cortar(agora);
    }

    /// Para o que estiver na saída, agora.
    fn cortar(&mut self, agora: Instant) {
        self.seguro.store(false, Ordering::Relaxed);
        self.passeio = Passeio::Fora;
        self.renovado_em = None;
        if let Some(mut som) = self.atual.take() {
            if !som.terminou() {
                som.cortar();
            }
            self.trava.terminou(agora);
        }
    }

    fn avisar_banco(&mut self) {
        if !self.avisou_banco {
            self.avisou_banco = true;
            tracing::warn!(banco = %self.banco.display(), "sem banco de voz: os sons são pulados (rode `sounds ensure-bank`)");
        }
    }

    /// Começo, laço enquanto o pedido vale, e fim, num fluxo só, para o laço emendar sem falha.
    fn comecar_wheee(&mut self, agora: Instant) {
        self.cortar(agora);
        let pasta = self.banco.join("wheee");
        let mut letras: Vec<String> = std::fs::read_dir(&pasta)
            .map(|rd| {
                rd.filter_map(|e| {
                    let nome = e.ok()?.file_name().into_string().ok()?;
                    Some(
                        nome.strip_prefix("wheee_start_")?
                            .strip_suffix(".wav")?
                            .to_owned(),
                    )
                })
                .collect()
            })
            .unwrap_or_default();
        letras.sort();
        let parte =
            |nome: &str, letra: &str| ler_wav_pcm(&pasta.join(format!("wheee_{nome}_{letra}.wav")));
        let partes = (!letras.is_empty())
            .then(|| letras.swap_remove(sorteio(letras.len())))
            .and_then(|l| Some((parte("start", &l)?, parte("loop", &l)?, parte("end", &l)?)));
        let Some(((hz, comeco), (_, laco), (_, fim))) = partes else {
            // Um banco sem as três partes: o som inteiro de uma vez, e o passeio marcado assim mesmo, ou cada renovação do
            // pedido, a cada 20 ms, abriria e mataria um tocador.
            self.tocar("wheee", agora);
            self.passeio = Passeio::Andando;
            return;
        };
        let (mut escrita, som) = match self.saida.abrir_fluxo(hz) {
            Ok(par) => par,
            Err(e) => {
                tracing::debug!(erro = %e, "a saída de som recusou o wheee");
                return;
            }
        };
        self.atual = Some(som);
        self.passeio = Passeio::Andando;
        self.trava.comecou();
        self.seguro = Arc::new(AtomicBool::new(true));
        let seguro = self.seguro.clone();
        let lancado = std::thread::Builder::new()
            .name("wheee".into())
            .spawn(move || {
                // Só 250 ms à frente do que está tocando: com mais que isso na fila, o fim começaria esse tanto depois de o
                // pedido parar.
                const PEDACO: usize = 8192;
                let bytes_por_s = f64::from(hz) * 2.0;
                let t0 = Instant::now();
                let mut mandados = 0usize;
                let mut mandar = |pedaco: &[u8]| -> bool {
                    let adiante = mandados as f64 / bytes_por_s - t0.elapsed().as_secs_f64();
                    if adiante > 0.25 {
                        std::thread::sleep(Duration::from_secs_f64(adiante - 0.25));
                    }
                    mandados += pedaco.len();
                    escrita.write_all(pedaco).is_ok()
                };
                for pedaco in comeco.chunks(PEDACO) {
                    if !mandar(pedaco) {
                        return;
                    }
                }
                'passeio: while seguro.load(Ordering::Relaxed) {
                    for pedaco in laco.chunks(PEDACO) {
                        if !mandar(pedaco) {
                            break 'passeio;
                        }
                    }
                }
                // Fora do laço com o fluxo aberto: o pedido venceu. Cortado, a primeira escrita falha e o fim não toca.
                for pedaco in fim.chunks(PEDACO) {
                    if !mandar(pedaco) {
                        return;
                    }
                }
            });
        if let Err(e) = lancado {
            tracing::error!(erro = %e, "sem thread para o wheee");
            self.cortar(agora);
        }
    }

    /// Se há som tocando agora.
    pub fn tocando(&self) -> bool {
        self.atual.is_some()
    }
}

/// Um número qualquer de 0 a `n - 1`: os nanossegundos do relógio, como o pato faz, que é o sorteio de que um som precisa.
fn sorteio(n: usize) -> usize {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);
    nanos % n
}

/// Um `.wav` qualquer da pasta de um som.
fn escolher(pasta: &Path) -> Option<PathBuf> {
    let mut arquivos: Vec<PathBuf> = std::fs::read_dir(pasta)
        .ok()?
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .collect();
    if arquivos.is_empty() {
        return None;
    }
    arquivos.sort();
    let i = sorteio(arquivos.len());
    Some(arquivos.swap_remove(i))
}

/// Lê um `.wav` de PCM: a taxa e as amostras cruas, S16LE. Só o que os arquivos mono de 16 bits do banco precisam.
fn ler_wav_pcm(caminho: &Path) -> Option<(u32, Vec<u8>)> {
    let b = std::fs::read(caminho).ok()?;
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let mut hz = 0u32;
    let mut pos = 12usize;
    while pos + 8 <= b.len() {
        let tamanho = u32::from_le_bytes(b[pos + 4..pos + 8].try_into().ok()?) as usize;
        let corpo = pos + 8;
        match &b[pos..pos + 4] {
            b"fmt " if corpo + 8 <= b.len() => {
                hz = u32::from_le_bytes(b[corpo + 4..corpo + 8].try_into().ok()?)
            }
            b"data" if hz > 0 => {
                return Some((hz, b[corpo..(corpo + tamanho).min(b.len())].to_vec()));
            }
            _ => {}
        }
        pos = corpo + tamanho + (tamanho & 1);
    }
    None
}

/// A saída do robô: um `aplay` por som, como no pato.
pub struct Aplay {
    pub dispositivo: String,
}

struct Processo(std::process::Child);

impl Tocando for Processo {
    fn terminou(&mut self) -> bool {
        !matches!(self.0.try_wait(), Ok(None))
    }
    fn cortar(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Saida for Aplay {
    fn tocar_arquivo(&mut self, wav: &Path) -> std::io::Result<Box<dyn Tocando>> {
        use std::process::{Command, Stdio};
        let filho = Command::new("aplay")
            .args(["-q", "-D", &self.dispositivo])
            .arg(wav)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(Box::new(Processo(filho)))
    }

    fn abrir_fluxo(
        &mut self,
        hz: u32,
    ) -> std::io::Result<(Box<dyn Write + Send>, Box<dyn Tocando>)> {
        use std::process::{Command, Stdio};
        let mut filho = Command::new("aplay")
            .args([
                "-q",
                "-D",
                &self.dispositivo,
                "-t",
                "raw",
                "-f",
                "S16_LE",
                "-c",
                "1",
            ])
            .args(["-r", &hz.to_string(), "--buffer-time=120000"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let entrada = filho
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("aplay sem entrada"))?;
        Ok((Box::new(entrada), Box::new(Processo(filho))))
    }
}

/// A saída nula: nada toca, e todo som termina na hora. Para o serviço sem alto-falante.
pub struct Nula;

struct Terminado;

impl Tocando for Terminado {
    fn terminou(&mut self) -> bool {
        true
    }
    fn cortar(&mut self) {}
}

impl Saida for Nula {
    fn tocar_arquivo(&mut self, _wav: &Path) -> std::io::Result<Box<dyn Tocando>> {
        Ok(Box::new(Terminado))
    }
    fn abrir_fluxo(
        &mut self,
        _hz: u32,
    ) -> std::io::Result<(Box<dyn Write + Send>, Box<dyn Tocando>)> {
        Ok((Box::new(std::io::sink()), Box::new(Terminado)))
    }
}

/// Os recados do alto-falante simulado ao serviço local: um byte com o tipo, quatro com o tamanho, e o conteúdo.
pub mod recado {
    /// A primeira coisa que vai pela conexão: quem liga é o alto-falante.
    pub const FALA: u8 = b'F';
    /// Um `.wav` inteiro, para tocar agora.
    pub const ARQUIVO: u8 = 1;
    /// Um fluxo começa: a taxa, em 4 bytes.
    pub const FLUXO: u8 = 2;
    /// Um pedaço do fluxo: PCM cru, S16LE, mono.
    pub const PEDACO: u8 = 3;
    /// O som para agora.
    pub const CORTE: u8 = 4;
}

/// A saída do robô simulado: cada som vai ao serviço local do simulador, que o entrega à página, e ela o toca. O som dura
/// aqui o que dura lá: é isso que mantém a trava contra a própria voz valendo no simulador.
pub struct Simulador {
    pub endereco: String,
}

fn mandar(conexao: &mut std::net::TcpStream, tipo: u8, conteudo: &[u8]) -> std::io::Result<()> {
    conexao.write_all(&[tipo])?;
    conexao.write_all(&(conteudo.len() as u32).to_le_bytes())?;
    conexao.write_all(conteudo)
}

/// Quanto dura um `.wav` de PCM: o tamanho dos dados sobre os bytes por segundo que o cabeçalho declara.
pub fn duracao_wav(b: &[u8]) -> Option<Duration> {
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let mut por_segundo = 0u32;
    let mut pos = 12usize;
    while pos + 8 <= b.len() {
        let tamanho = u32::from_le_bytes(b[pos + 4..pos + 8].try_into().ok()?) as usize;
        let corpo = pos + 8;
        match &b[pos..pos + 4] {
            b"fmt " if corpo + 12 <= b.len() => {
                por_segundo = u32::from_le_bytes(b[corpo + 8..corpo + 12].try_into().ok()?)
            }
            b"data" if por_segundo > 0 => {
                let dados = tamanho.min(b.len() - corpo);
                return Some(Duration::from_secs_f64(
                    dados as f64 / f64::from(por_segundo),
                ));
            }
            _ => {}
        }
        pos = corpo + tamanho + (tamanho & 1);
    }
    None
}

impl Simulador {
    fn ligar(&self) -> std::io::Result<std::net::TcpStream> {
        let mut conexao = std::net::TcpStream::connect(&self.endereco)?;
        let _ = conexao.set_nodelay(true);
        conexao.write_all(&[recado::FALA])?;
        Ok(conexao)
    }
}

/// Um arquivo tocando na página: termina quando a duração dele passa.
struct NaPagina {
    conexao: std::net::TcpStream,
    fim: Instant,
}

impl Tocando for NaPagina {
    fn terminou(&mut self) -> bool {
        Instant::now() >= self.fim
    }
    fn cortar(&mut self) {
        let _ = mandar(&mut self.conexao, recado::CORTE, &[]);
    }
}

/// O que quem escreve o fluxo e quem o vigia partilham.
struct Fluxo {
    hz: u32,
    comecou: Instant,
    bytes: std::sync::atomic::AtomicUsize,
    fechado: AtomicBool,
    cortado: AtomicBool,
}

struct EscritaDoFluxo {
    conexao: std::net::TcpStream,
    fluxo: Arc<Fluxo>,
}

impl Write for EscritaDoFluxo {
    fn write(&mut self, pedaco: &[u8]) -> std::io::Result<usize> {
        if self.fluxo.cortado.load(Ordering::Relaxed) {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        mandar(&mut self.conexao, recado::PEDACO, pedaco)?;
        self.fluxo.bytes.fetch_add(pedaco.len(), Ordering::Relaxed);
        Ok(pedaco.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.conexao.flush()
    }
}

impl Drop for EscritaDoFluxo {
    fn drop(&mut self) {
        self.fluxo.fechado.store(true, Ordering::Relaxed);
    }
}

struct FluxoNaPagina {
    conexao: std::net::TcpStream,
    fluxo: Arc<Fluxo>,
}

impl Tocando for FluxoNaPagina {
    fn terminou(&mut self) -> bool {
        // Quem escreve já saiu, e o que foi mandado já tocou.
        let tocado = Duration::from_secs_f64(
            self.fluxo.bytes.load(Ordering::Relaxed) as f64 / (f64::from(self.fluxo.hz) * 2.0),
        );
        self.fluxo.fechado.load(Ordering::Relaxed) && self.fluxo.comecou.elapsed() >= tocado
    }
    fn cortar(&mut self) {
        self.fluxo.cortado.store(true, Ordering::Relaxed);
        let _ = mandar(&mut self.conexao, recado::CORTE, &[]);
    }
}

impl Saida for Simulador {
    fn tocar_arquivo(&mut self, wav: &Path) -> std::io::Result<Box<dyn Tocando>> {
        let bytes = std::fs::read(wav)?;
        let dura = duracao_wav(&bytes).ok_or_else(|| {
            std::io::Error::other(format!("{} não é um wav de PCM", wav.display()))
        })?;
        let mut conexao = self.ligar()?;
        mandar(&mut conexao, recado::ARQUIVO, &bytes)?;
        Ok(Box::new(NaPagina {
            conexao,
            fim: Instant::now() + dura,
        }))
    }

    fn abrir_fluxo(
        &mut self,
        hz: u32,
    ) -> std::io::Result<(Box<dyn Write + Send>, Box<dyn Tocando>)> {
        let mut conexao = self.ligar()?;
        mandar(&mut conexao, recado::FLUXO, &hz.to_le_bytes())?;
        let fluxo = Arc::new(Fluxo {
            hz: hz.max(1),
            comecou: Instant::now(),
            bytes: std::sync::atomic::AtomicUsize::new(0),
            fechado: AtomicBool::new(false),
            cortado: AtomicBool::new(false),
        });
        let escrita = EscritaDoFluxo {
            conexao: conexao.try_clone()?,
            fluxo: fluxo.clone(),
        };
        Ok((
            Box::new(escrita),
            Box::new(FluxoNaPagina { conexao, fluxo }),
        ))
    }
}
