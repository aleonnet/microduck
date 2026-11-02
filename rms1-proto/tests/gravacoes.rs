//! O protocolo contra o robô. Cada arquivo foi lido da UART do S1 ou é o quadro exato que o moveu.

use rms1_proto::crc::{crc8, crc16};
use rms1_proto::{Leitor, Quadro, QuadroErro};

const BATIMENTO: &[u8] = include_bytes!("../../s1-duml/tests/fixtures/heartbeat_real.bin");
const FLUXO: &[u8] = include_bytes!("../../s1-duml/tests/fixtures/ttyS3_idle_read_stream_4k.bin");
const TELEMETRIA: &[u8] = include_bytes!("../../s1-duml/tests/fixtures/dds_push_176_real.bin");
const TORRE: &[u8] = include_bytes!("../../s1-duml/tests/fixtures/poc_gim.bin");
const RODAS: &[u8] = include_bytes!("../../s1-duml/tests/fixtures/poc_fwd.bin");
const MODO: &[u8] = include_bytes!("../../s1-duml/tests/fixtures/poc_wm.bin");

/// Um verificador errado faz o corpo ignorar tudo em silêncio.
#[test]
fn os_verificadores_batem_com_o_quadro_do_robo() {
    assert_eq!(crc8(&BATIMENTO[..3]), BATIMENTO[3]);
    let n = BATIMENTO.len();
    assert_eq!(
        crc16(&BATIMENTO[..n - 2]).to_le_bytes(),
        [BATIMENTO[n - 2], BATIMENTO[n - 1]]
    );
}

/// Ler e remontar tem de dar os mesmos bytes: ordem dos campos, tamanho e verificadores são os do robô.
#[test]
fn quadros_gravados_voltam_byte_a_byte() {
    for gravado in [BATIMENTO, TELEMETRIA, TORRE, RODAS, MODO] {
        let q = Quadro::ler(gravado).expect("o quadro do robô é válido");
        assert_eq!(q.montar().unwrap(), gravado);
    }
}

/// Um fluxo real, começando no meio de um quadro: todo quadro achado volta igual, em ordem.
#[test]
fn o_fluxo_gravado_da_uart_vira_quadros_e_volta_igual() {
    let mut l = Leitor::new();
    let mut quadros = Vec::new();
    for pedaco in FLUXO.chunks(37) {
        quadros.extend(l.empurrar(pedaco));
    }
    assert!(quadros.len() > 10, "achou só {} quadros", quadros.len());
    let mut pos = 0;
    for q in &quadros {
        let b = q.montar().unwrap();
        let achado = FLUXO[pos..]
            .windows(b.len())
            .position(|w| w == b.as_slice())
            .expect("cada quadro está no fluxo, na ordem");
        pos += achado + b.len();
    }
}

#[test]
fn quadro_alterado_e_recusado() {
    let mut b = BATIMENTO.to_vec();
    b[12] ^= 0x01;
    assert!(matches!(Quadro::ler(&b), Err(QuadroErro::Crc16 { .. })));
    let mut b = BATIMENTO.to_vec();
    b[1] ^= 0x01;
    assert!(matches!(Quadro::ler(&b), Err(QuadroErro::Crc8 { .. })));
}

/// Um byte corrompido custa um quadro, nunca o resto.
#[test]
fn lixo_no_meio_custa_so_o_quadro_atingido() {
    let mut fluxo = BATIMENTO.to_vec();
    fluxo.extend_from_slice(&[0x55, 0x55, 0x01]);
    let mut ruim = BATIMENTO.to_vec();
    ruim[15] ^= 0xff;
    fluxo.extend_from_slice(&ruim);
    fluxo.extend_from_slice(BATIMENTO);
    let mut l = Leitor::new();
    let quadros = l.empurrar(&fluxo);
    assert_eq!(quadros.len(), 2);
    assert!(l.descartados() > 0);
}
