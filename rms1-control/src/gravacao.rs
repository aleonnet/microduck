//! Gravar o que foi enviado ao corpo e comparar duas gravações, para provar que simulador e robô
//! receberam os mesmos quadros. E gravar o que o corpo devolveu, para reprocessar sem o robô: cada problema no robô vira
//! dado gravado, e não bancada.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::time::Instant;

use rms1_proto::Quadro;
use rms1_proto::barramento;
use rms1_proto::comando::id;

use crate::corpo::Corpo;

/// Linhas escritas entre duas descargas do arquivo.
const DESCARGA: u32 = 64;

/// Por fora de qualquer corpo: grava e repassa. Escrita em memória, descarregada a cada 64 linhas.
pub struct Gravando<C: Corpo> {
    corpo: C,
    arquivo: BufWriter<File>,
    inicio: Instant,
    pendentes: u32,
    /// O que o corpo devolveu, `ms hex` por leitura, no mesmo relógio do enviado.
    recebido: Option<(BufWriter<File>, u32)>,
    /// A gravação do recebido está falhando: o aviso sai uma vez por falha, e não a cada leitura.
    recebido_falhando: bool,
}

impl<C: Corpo> Gravando<C> {
    pub fn new(corpo: C, caminho: &Path) -> io::Result<Self> {
        Ok(Self {
            corpo,
            arquivo: BufWriter::new(File::create(caminho)?),
            inicio: Instant::now(),
            pendentes: 0,
            recebido: None,
            recebido_falhando: false,
        })
    }

    /// Grava também cada leitura do corpo, em arquivo próprio.
    pub fn com_recebido(mut self, caminho: &Path) -> io::Result<Self> {
        self.recebido = Some((BufWriter::new(File::create(caminho)?), 0));
        Ok(self)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl<C: Corpo> Corpo for Gravando<C> {
    fn enviar(&mut self, bytes: &[u8]) -> io::Result<()> {
        let r = self.corpo.enviar(bytes);
        // O que falhou no envio não chegou ao corpo: não entra na gravação.
        if r.is_ok() {
            writeln!(
                self.arquivo,
                "{} {}",
                self.inicio.elapsed().as_millis(),
                hex(bytes)
            )?;
            self.pendentes += 1;
            if self.pendentes >= DESCARGA {
                self.arquivo.flush()?;
                self.pendentes = 0;
            }
        }
        r
    }

    fn ler(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.corpo.ler(buf)?;
        if n > 0
            && let Some((arquivo, pendentes)) = &mut self.recebido
        {
            // Falha ao gravar não pode tirar a leitura do laço: o que chegou segue para a telemetria.
            let gravou = writeln!(
                arquivo,
                "{} {}",
                self.inicio.elapsed().as_millis(),
                hex(&buf[..n])
            );
            *pendentes += 1;
            if gravou.is_err() || *pendentes >= DESCARGA {
                match gravou.and_then(|()| arquivo.flush()) {
                    Err(e) if !self.recebido_falhando => {
                        tracing::warn!(erro = %e, "gravar o recebido falhou");
                        self.recebido_falhando = true;
                    }
                    Err(_) => {}
                    Ok(()) => self.recebido_falhando = false,
                }
                *pendentes = 0;
            }
        }
        Ok(n)
    }
}

impl<C: Corpo> Drop for Gravando<C> {
    fn drop(&mut self) {
        let _ = self.arquivo.flush();
        if let Some((arquivo, _)) = &mut self.recebido {
            let _ = arquivo.flush();
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
        // A telemetria depende do que o corpo devolve, e o corpo falso não devolve nada: a comparação é dos comandos.
        if q.cmd_set == barramento::id::PACOTE.0 {
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
