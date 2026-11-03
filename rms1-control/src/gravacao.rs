//! Gravar o que foi enviado ao corpo e comparar duas gravações, para provar que simulador e robô
//! receberam os mesmos quadros.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::time::Instant;

use rms1_proto::Quadro;
use rms1_proto::barramento;
use rms1_proto::comando::id;

use crate::corpo::Corpo;

/// Por fora de qualquer corpo: grava e repassa. Escrita em memória, descarregada a cada 64 quadros.
pub struct Gravando<C: Corpo> {
    corpo: C,
    arquivo: BufWriter<File>,
    inicio: Instant,
    pendentes: u32,
}

impl<C: Corpo> Gravando<C> {
    pub fn new(corpo: C, caminho: &Path) -> io::Result<Self> {
        Ok(Self {
            corpo,
            arquivo: BufWriter::new(File::create(caminho)?),
            inicio: Instant::now(),
            pendentes: 0,
        })
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
            if self.pendentes >= 64 {
                self.arquivo.flush()?;
                self.pendentes = 0;
            }
        }
        r
    }

    fn ler(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.corpo.ler(buf)
    }
}

impl<C: Corpo> Drop for Gravando<C> {
    fn drop(&mut self) {
        let _ = self.arquivo.flush();
    }
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
        let bytes: Vec<u8> = (0..h.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&h[i..i + 2], 16))
            .collect::<Result<_, _>>()
            .map_err(io::Error::other)?;
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
