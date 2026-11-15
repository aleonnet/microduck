//! Gravar o que foi enviado ao corpo e comparar duas gravações, para provar que simulador e robô
//! receberam os mesmos quadros. E gravar o que o corpo devolveu, para reprocessar sem o robô: cada problema no robô vira
//! dado gravado, e não bancada.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::Instant;

use rms1_proto::Quadro;
use rms1_proto::barramento;
use rms1_proto::comando::id;

use crate::corpo::Corpo;

/// Linhas escritas entre duas descargas do arquivo.
const DESCARGA: u32 = 64;
/// Linhas que esperam a escritora. O robô grava cerca de 81 por segundo (4.449 linhas em 54,8 s, medido em 2026-11-15): são
/// uns 10 s de disco parado antes de a primeira linha ser descartada.
const FILA: usize = 1024;
/// O limite dos dois arquivos somados, quando ninguém diz outro: o padrão de `corpo.gravar_limite_mb`.
const LIMITE_PADRAO: u64 = 64 * 1024 * 1024;

/// Para onde a gravação escreve: um arquivo, ou o disco de mentira de um teste.
pub type Destino = Box<dyn Write + Send>;

#[derive(Clone, Copy)]
enum Qual {
    Enviado,
    Recebido,
}

enum Recado {
    Linha(Qual, String),
    Recebido(Destino),
    Limite(u64),
}

/// Por fora de qualquer corpo: grava e repassa.
///
/// A escrita não é feita no ciclo. Em 2026-11-15 o disco do robô encheu, a escrita era feita aqui, na thread do laço, e o laço
/// caiu de 100 para 2 a 4 Hz (`out/2026-11-14-s1-midia/medicao-disco-do-robo.txt`, no laboratório). O ciclo só monta a linha
/// e a entrega a uma fila limitada, sem esperar; uma thread só dela escreve e descarrega a cada 64 linhas. Fila cheia, a linha
/// inteira é descartada e contada. Falha do disco fica com a escritora: o quadro foi ao corpo, e o envio diz isso.
pub struct Gravando<C: Corpo> {
    corpo: C,
    inicio: Instant,
    fila: Option<SyncSender<Recado>>,
    escritora: Option<JoinHandle<()>>,
    grava_recebido: bool,
    perdidas: Arc<AtomicU64>,
    /// Há linhas sendo descartadas: o aviso sai uma vez por episódio, e não a cada linha.
    perdendo: bool,
}

impl<C: Corpo> Gravando<C> {
    pub fn new(corpo: C, caminho: &Path) -> io::Result<Self> {
        Ok(Self::com_escritores(
            corpo,
            Box::new(File::create(caminho)?),
            None,
        ))
    }

    /// Com os destinos dados, em vez de arquivos: é como um teste põe um disco lento ou cheio no lugar.
    pub fn com_escritores(corpo: C, enviado: Destino, recebido: Option<Destino>) -> Self {
        let (fila, recados) = mpsc::sync_channel(FILA);
        let grava_recebido = recebido.is_some();
        let escritora = std::thread::Builder::new()
            .name("gravacao".to_owned())
            .spawn(move || escrever(&recados, enviado, recebido));
        let (fila, escritora) = match escritora {
            Ok(e) => (Some(fila), Some(e)),
            Err(e) => {
                tracing::warn!(erro = %e, "a gravação não começou: sem a thread escritora");
                (None, None)
            }
        };
        Self {
            corpo,
            inicio: Instant::now(),
            fila,
            escritora,
            grava_recebido,
            perdidas: Arc::new(AtomicU64::new(0)),
            perdendo: false,
        }
    }

    /// Grava também cada leitura do corpo, em arquivo próprio.
    pub fn com_recebido(mut self, caminho: &Path) -> io::Result<Self> {
        let destino: Destino = Box::new(File::create(caminho)?);
        // Na montagem, antes de qualquer ciclo: aqui pode esperar a fila.
        if let Some(f) = &self.fila
            && f.send(Recado::Recebido(destino)).is_ok()
        {
            self.grava_recebido = true;
        }
        Ok(self)
    }

    /// O limite dos dois arquivos somados, em bytes. Chegando nele, a gravação para, avisa uma vez, e o laço segue.
    pub fn com_limite(self, bytes: u64) -> Self {
        if let Some(f) = &self.fila {
            let _ = f.send(Recado::Limite(bytes));
        }
        self
    }

    /// Quantas linhas foram descartadas porque a escritora não acompanhou.
    pub fn perdidas(&self) -> Arc<AtomicU64> {
        self.perdidas.clone()
    }

    /// Entrega uma linha à escritora, sem esperar. É o único ponto em que o ciclo toca a gravação.
    fn entregar(&mut self, qual: Qual, bytes: &[u8]) {
        let Some(fila) = &self.fila else {
            return;
        };
        let linha = format!("{} {}\n", self.inicio.elapsed().as_millis(), hex(bytes));
        match fila.try_send(Recado::Linha(qual, linha)) {
            Ok(()) => self.perdendo = false,
            Err(TrySendError::Full(_)) => {
                let n = self.perdidas.fetch_add(1, Ordering::Relaxed) + 1;
                if !self.perdendo {
                    tracing::warn!(
                        perdidas = n,
                        "a gravação não acompanha o laço: linhas descartadas"
                    );
                    self.perdendo = true;
                }
            }
            Err(TrySendError::Disconnected(_)) => {
                tracing::warn!("a escritora da gravação saiu: nada mais é gravado");
                self.fila = None;
            }
        }
    }
}

/// A thread escritora: a única que toca o disco. Sai quando a fila fecha, depois de escrever o que estava nela.
fn escrever(recados: &Receiver<Recado>, enviado: Destino, recebido: Option<Destino>) {
    let mut enviado = (BufWriter::new(enviado), 0u32);
    let mut recebido = recebido.map(|r| (BufWriter::new(r), 0u32));
    let mut limite = LIMITE_PADRAO;
    let mut escritos = 0u64;
    let mut no_limite = false;
    // A escrita está falhando: o aviso sai uma vez por falha, e não a cada linha.
    let mut falhando = false;
    for recado in recados {
        let (qual, linha) = match recado {
            Recado::Recebido(d) => {
                recebido = Some((BufWriter::new(d), 0));
                continue;
            }
            Recado::Limite(n) => {
                limite = n;
                continue;
            }
            Recado::Linha(qual, linha) => (qual, linha),
        };
        if no_limite {
            continue;
        }
        if escritos + linha.len() as u64 > limite {
            no_limite = true;
            tracing::warn!(
                limite_bytes = limite,
                "a gravação chegou ao limite e parou; o laço segue"
            );
            continue;
        }
        let Some((arquivo, pendentes)) = (match qual {
            Qual::Enviado => Some(&mut enviado),
            Qual::Recebido => recebido.as_mut(),
        }) else {
            continue;
        };
        let mut gravou = arquivo.write_all(linha.as_bytes());
        *pendentes += 1;
        if gravou.is_ok() && *pendentes >= DESCARGA {
            *pendentes = 0;
            gravou = arquivo.flush();
        }
        match gravou {
            Ok(()) => {
                escritos += linha.len() as u64;
                falhando = false;
            }
            Err(e) if !falhando => {
                tracing::warn!(erro = %e, "gravar falhou");
                falhando = true;
            }
            Err(_) => {}
        }
    }
    let _ = enviado.0.flush();
    if let Some((arquivo, _)) = &mut recebido {
        let _ = arquivo.flush();
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl<C: Corpo> Corpo for Gravando<C> {
    fn enviar(&mut self, bytes: &[u8]) -> io::Result<()> {
        let r = self.corpo.enviar(bytes);
        // O que falhou no envio não chegou ao corpo: não entra na gravação. E o que a gravação fizer não muda a resposta do corpo.
        if r.is_ok() {
            self.entregar(Qual::Enviado, bytes);
        }
        r
    }

    fn ler(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.corpo.ler(buf)?;
        if n > 0 && self.grava_recebido {
            self.entregar(Qual::Recebido, &buf[..n]);
        }
        Ok(n)
    }
}

impl<C: Corpo> Drop for Gravando<C> {
    /// Fecha a fila e espera a escritora: o que estava na fila vai ao disco, e o fim do arquivo é o último quadro enviado, o
    /// comando de parar de quem sai armado.
    fn drop(&mut self) {
        drop(self.fila.take());
        if let Some(e) = self.escritora.take() {
            let _ = e.join();
        }
        let perdidas = self.perdidas.load(Ordering::Relaxed);
        if perdidas > 0 {
            tracing::warn!(perdidas, "a gravação terminou com linhas descartadas");
        }
    }
}

/// As leituras de uma gravação do recebido, na ordem em que chegaram: os milissegundos desde o início e os bytes.
pub fn ler_recebido(caminho: &Path) -> io::Result<Vec<(u64, Vec<u8>)>> {
    let mut leituras = Vec::new();
    for linha in BufReader::new(File::open(caminho)?).lines() {
        let linha = linha?;
        let Some((ms, h)) = linha.split_once(' ') else {
            continue;
        };
        let ms = ms
            .parse()
            .map_err(|e| io::Error::other(format!("milissegundos `{ms}`: {e}")))?;
        leituras.push((ms, de_hex(h)?));
    }
    Ok(leituras)
}

fn de_hex(h: &str) -> io::Result<Vec<u8>> {
    if !h.len().is_multiple_of(2) {
        return Err(io::Error::other(format!("hexadecimal ímpar: {h}")));
    }
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16))
        .collect::<Result<_, _>>()
        .map_err(io::Error::other)
}

/// `(cmd_set, cmd_id)` para a sequência de `(origem, destino, dados)`.
pub type Resumo = BTreeMap<(u8, u8), Vec<(u8, u8, Vec<u8>)>>;

/// Por comando, a sequência de conteúdos, sem repetições seguidas e sem número de sequência. Fica de
/// fora quanto tempo cada um durou, que muda de uma execução para outra.
pub fn resumo(caminho: &Path) -> io::Result<Resumo> {
    let mut saida = Resumo::new();
    for linha in BufReader::new(File::open(caminho)?).lines() {
        let linha = linha?;
        let Some((_, h)) = linha.split_once(' ') else {
            continue;
        };
        let bytes = de_hex(h)?;
        let q = Quadro::ler(&bytes).map_err(io::Error::other)?;
        // A telemetria depende do que o corpo devolve, e o corpo falso não devolve nada: a comparação é dos comandos. A
        // sensibilidade das placas também: ela sai com o primeiro pacote (`Laco::ciclo`).
        if q.cmd_set == barramento::id::PACOTE.0 || (q.cmd_set, q.cmd_id) == id::ARMADURA_VOZ {
            continue;
        }
        let lista = saida.entry((q.cmd_set, q.cmd_id)).or_default();
        let mut dados = q.dados;
        // O identificador de tarefa muda a cada envio, como na DJI: conta quantos envios, e não o que foi pedido.
        if (q.cmd_set, q.cmd_id) == id::TORRE_ANGULOS && !dados.is_empty() {
            dados[0] = 0;
        }
        let item = (q.origem, q.destino, dados);
        if lista.last() != Some(&item) {
            lista.push(item);
        }
    }
    Ok(saida)
}

/// As diferenças entre duas gravações. Vazio é igual. Gravação sem nenhum quadro é erro: dois nadas iguais não
/// provam nada.
pub fn diferencas(a: &Path, b: &Path) -> io::Result<Vec<String>> {
    let (ra, rb) = (resumo(a)?, resumo(b)?);
    for (r, caminho) in [(&ra, a), (&rb, b)] {
        if r.is_empty() {
            return Err(io::Error::other(format!(
                "{} não tem nenhum quadro",
                caminho.display()
            )));
        }
    }
    let mut d = Vec::new();
    for cmd in ra
        .keys()
        .chain(rb.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let (la, lb) = (ra.get(cmd), rb.get(cmd));
        if la != lb {
            d.push(format!(
                "0x{:02x}/0x{:02x}: {} contra {} conteúdos",
                cmd.0,
                cmd.1,
                la.map_or(0, Vec::len),
                lb.map_or(0, Vec::len)
            ));
        }
    }
    Ok(d)
}
