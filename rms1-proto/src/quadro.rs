//! Um quadro DUML/V1.
//!
//! ```text
//! 0     1        2                3     4       5        6 7  8      9       10      11..   fim-2 fim-1
//! 0x55  tamanho  4 | tamanho>>8   crc8  origem  destino  seq  attri  cmd_set cmd_id  dados  crc16
//! ```
//!
//! `tamanho` é o quadro inteiro. `attri` é `resposta << 7 | pede << 5`.

use crate::crc::{crc8, crc16};

pub const INICIO: u8 = 0x55;
/// Cabeçalho mais os dois verificadores: um quadro sem dados tem este tamanho.
pub const SOBRA: usize = 13;
const POS_DADOS: usize = 11;
const TAMANHO_MAX: usize = 0x3ff;

/// O que o remetente quer de volta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PedeAck {
    #[default]
    Nada = 0,
    Agora = 1,
    AoTerminar = 2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quadro {
    pub origem: u8,
    pub destino: u8,
    pub seq: u16,
    pub resposta: bool,
    pub pede: PedeAck,
    pub cmd_set: u8,
    pub cmd_id: u8,
    pub dados: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuadroErro {
    #[error("quadro curto: {tem} bytes, precisa de {precisa}")]
    Curto { tem: usize, precisa: usize },
    #[error("não começa com 0x55: 0x{0:02x}")]
    Inicio(u8),
    #[error("crc8 errado: calculado 0x{calculado:02x}, no quadro 0x{lido:02x}")]
    Crc8 { calculado: u8, lido: u8 },
    #[error("crc16 errado: calculado 0x{calculado:04x}, no quadro 0x{lido:04x}")]
    Crc16 { calculado: u16, lido: u16 },
    #[error("tamanho declarado {0} é menor que o de um quadro vazio")]
    TamanhoDeclarado(usize),
    #[error("{0} bytes de dados não cabem num quadro")]
    Longo(usize),
    #[error("attri 0x{0:02x} tem os dois bits de pedido ligados")]
    Attri(u8),
}

impl Quadro {
    pub fn tamanho(&self) -> usize {
        self.dados.len() + SOBRA
    }

    pub fn montar(&self) -> Result<Vec<u8>, QuadroErro> {
        let n = self.tamanho();
        if n > TAMANHO_MAX {
            return Err(QuadroErro::Longo(self.dados.len()));
        }
        let mut b = Vec::with_capacity(n);
        b.push(INICIO);
        b.push((n & 0xff) as u8);
        b.push(((n >> 8) as u8 & 0x3) | 0x4);
        b.push(crc8(&b[..3]));
        b.push(self.origem);
        b.push(self.destino);
        b.extend_from_slice(&self.seq.to_le_bytes());
        b.push((u8::from(self.resposta) << 7) | ((self.pede as u8) << 5));
        b.push(self.cmd_set);
        b.push(self.cmd_id);
        b.extend_from_slice(&self.dados);
        let c = crc16(&b);
        b.extend_from_slice(&c.to_le_bytes());
        Ok(b)
    }

    /// Um quadro inteiro. Bytes depois do tamanho declarado são ignorados.
    pub fn ler(b: &[u8]) -> Result<Self, QuadroErro> {
        if b.len() < SOBRA {
            return Err(QuadroErro::Curto {
                tem: b.len(),
                precisa: SOBRA,
            });
        }
        if b[0] != INICIO {
            return Err(QuadroErro::Inicio(b[0]));
        }
        let calculado = crc8(&b[..3]);
        if calculado != b[3] {
            return Err(QuadroErro::Crc8 {
                calculado,
                lido: b[3],
            });
        }
        let n = usize::from(b[1]) | (usize::from(b[2] & 0x3) << 8);
        if n < SOBRA {
            return Err(QuadroErro::TamanhoDeclarado(n));
        }
        if b.len() < n {
            return Err(QuadroErro::Curto {
                tem: b.len(),
                precisa: n,
            });
        }
        let calculado = crc16(&b[..n - 2]);
        let lido = u16::from_le_bytes([b[n - 2], b[n - 1]]);
        if calculado != lido {
            return Err(QuadroErro::Crc16 { calculado, lido });
        }
        let pede = match (b[8] >> 5) & 0x3 {
            0 => PedeAck::Nada,
            1 => PedeAck::Agora,
            2 => PedeAck::AoTerminar,
            _ => return Err(QuadroErro::Attri(b[8])),
        };
        Ok(Self {
            origem: b[4],
            destino: b[5],
            seq: u16::from_le_bytes([b[6], b[7]]),
            resposta: b[8] & 0x80 != 0,
            pede,
            cmd_set: b[9],
            cmd_id: b[10],
            dados: b[POS_DADOS..n - 2].to_vec(),
        })
    }
}

/// Quadros de um fluxo de bytes. Um byte corrompido custa um quadro, nunca o resto da sessão: procura o
/// próximo `0x55` com cabeçalho válido, espera o tamanho declarado, confere, e se falhar anda um byte.
#[derive(Debug, Default)]
pub struct Leitor {
    buf: Vec<u8>,
    descartados: u64,
}

impl Leitor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn descartados(&self) -> u64 {
        self.descartados
    }

    pub fn empurrar(&mut self, bytes: &[u8]) -> Vec<Quadro> {
        self.buf.extend_from_slice(bytes);
        let mut saida = Vec::new();
        let mut pos = 0;
        loop {
            let Some(rel) = self.buf[pos..].iter().position(|&b| b == INICIO) else {
                self.descartados += (self.buf.len() - pos) as u64;
                pos = self.buf.len();
                break;
            };
            self.descartados += rel as u64;
            pos += rel;
            let resto = &self.buf[pos..];
            if resto.len() < 4 {
                break;
            }
            if crc8(&resto[..3]) != resto[3] {
                self.descartados += 1;
                pos += 1;
                continue;
            }
            let n = usize::from(resto[1]) | (usize::from(resto[2] & 0x3) << 8);
            if n < SOBRA {
                self.descartados += 1;
                pos += 1;
                continue;
            }
            if resto.len() < n {
                break;
            }
            match Quadro::ler(&resto[..n]) {
                Ok(q) => {
                    saida.push(q);
                    pos += n;
                }
                Err(_) => {
                    self.descartados += 1;
                    pos += 1;
                }
            }
        }
        self.buf.drain(..pos);
        saida
    }
}
