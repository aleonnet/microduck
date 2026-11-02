//! Os dois verificadores de todo quadro.
//!
//! Sementes do `algo.py` da DJI: `crc8_calc(data, crc=0x77)` e `crc16_calc(data, crc=0x3692)`. As tabelas
//! são geradas dos polinômios refletidos `0x8C` e `0x8408`; `tests/gravacoes.rs` confere o resultado
//! contra os verificadores de quadros lidos do robô.

pub const SEMENTE8: u8 = 0x77;
pub const SEMENTE16: u16 = 0x3692;

const fn tabela8() -> [u8; 256] {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u8;
        let mut b = 0;
        while b < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0x8C } else { c >> 1 };
            b += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

const fn tabela16() -> [u16; 256] {
    let mut t = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u16;
        let mut b = 0;
        while b < 8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0x8408
            } else {
                c >> 1
            };
            b += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

const TABELA8: [u8; 256] = tabela8();
const TABELA16: [u16; 256] = tabela16();

pub fn crc8(dados: &[u8]) -> u8 {
    dados
        .iter()
        .fold(SEMENTE8, |c, &d| TABELA8[usize::from(c ^ d)])
}

pub fn crc16(dados: &[u8]) -> u16 {
    dados.iter().fold(SEMENTE16, |c, &d| {
        (c >> 8) ^ TABELA16[usize::from((c ^ u16::from(d)) as u8)]
    })
}
