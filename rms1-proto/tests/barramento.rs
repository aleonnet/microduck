//! O formato da telemetria contra o que a DJI e o robô fizeram, byte a byte.

use rms1_proto::Quadro;
use rms1_proto::barramento::{
    self, ASSINATURA, BATERIA, Dado, FREQUENCIA_HZ, Layout, MENSAGEM, POSICAO, Pacote, Pedido,
    RODAS, TORRE_BASICO, VELOCIDADE, id,
};

const REGISTRO: &[u8] = include_bytes!("../../s1-duml/tests/fixtures/dds_add_node_stock_real.bin");
const ASSINATURA_1HZ: &[u8] =
    include_bytes!("../../s1-duml/tests/fixtures/dds_add_sub_stock_real.bin");
const RESPOSTA: &[u8] = include_bytes!("../../s1-duml/tests/fixtures/dds_push_ack_stock_real.bin");
const PARADO: &[u8] =
    include_bytes!("../../s1-duml/tests/fixtures/dds_push_stock_idle_2026-10-30.bin");
const ANDANDO: &[u8] =
    include_bytes!("../../s1-duml/tests/fixtures/dds_push_stock_moving_2026-10-30.bin");

fn bytes(h: &str) -> Vec<u8> {
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
        .collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Os três quadros que a DJI manda ao ligar, lidos da serial.
#[test]
fn registro_assinatura_e_resposta_sao_os_gravados() {
    let g = Quadro::ler(REGISTRO).unwrap();
    assert_eq!(
        Pedido::RegistrarNo.quadro(g.seq).montar().unwrap(),
        REGISTRO,
        "registro"
    );
    const BATERIA_SO: [Dado; 1] = [BATERIA];
    let g = Quadro::ler(ASSINATURA_1HZ).unwrap();
    let p = Pedido::Assinar {
        mensagem: 0,
        dados: &BATERIA_SO,
        frequencia: 1,
    };
    assert_eq!(
        p.quadro(g.seq).montar().unwrap(),
        ASSINATURA_1HZ,
        "assinatura de 1 Hz"
    );
    let g = Quadro::ler(RESPOSTA).unwrap();
    assert_eq!(
        Pedido::Responder { seq: g.seq }.quadro(0).montar().unwrap(),
        RESPOSTA,
        "resposta ao pacote"
    );
}

/// `hdvt_exec_writes.strace`: `00 09 00`, de 09 para 03, atributo a40.
#[test]
fn cancelar_tem_os_dados_gravados() {
    let q = Pedido::Cancelar { mensagem: 0 }.quadro(7);
    assert_eq!(q.dados, vec![0, 9, 0]);
    assert_eq!(
        (q.origem, q.destino, q.cmd_set, q.cmd_id),
        (0x09, 0x03, 0x48, 0x04)
    );
    assert_eq!(q.montar().unwrap()[8], 0x40);
}

/// Dois quadros de 50 Hz de outros robôs: provam o formato, e não que este firmware aceita.
#[test]
fn quadros_de_50_hz_de_terceiros_voltam_byte_a_byte() {
    const DOIS: [Dado; 2] = [TORRE_BASICO, VELOCIDADE];
    // `lab201.csv`, linha 108, de `robomaster_s1_can_hack`.
    let lab = bytes("552404400903040040480309010300" /* */);
    let lab = [lab, bytes("02a702298803000200663e3e4c030002003200854c")].concat();
    let p = Pedido::Assinar {
        mensagem: 1,
        dados: &DOIS,
        frequencia: 50,
    };
    assert_eq!(hex(&p.quadro(4).montar().unwrap()), hex(&lab));

    // `topic_view.rs:test_subscribe_add_parse`, de `robomaster-s1-proto`.
    const SETE: [Dado; 7] = [
        TORRE_BASICO,
        VELOCIDADE,
        BATERIA,
        RODAS,
        barramento::IMU,
        barramento::ATITUDE,
        POSICAO,
    ];
    let topic = bytes(
        "554c046c09038d0440480309010300\
         07a702298803000200663e3e4c03000200fbdcf5d70300020009a326e203000200\
         f41d1cdc0300020042ee131d03000200b3f7e647030002003200 54bb"
            .replace(' ', "")
            .as_str(),
    );
    let p = Pedido::Assinar {
        mensagem: 1,
        dados: &SETE,
        frequencia: 50,
    };
    assert_eq!(hex(&p.quadro(0x048d).montar().unwrap()), hex(&topic));
}

/// A reconstrução de `assinatura.py`, no laboratório, com a ordem de `s2.strace`.
#[test]
fn a_assinatura_de_50_hz_e_a_reconstruida() {
    let p = Pedido::Assinar {
        mensagem: MENSAGEM,
        dados: &ASSINATURA,
        frequencia: FREQUENCIA_HZ,
    };
    assert_eq!(
        hex(&p.quadro(0).dados),
        "0901030009a702298803000200663e3e4c03000200fbdcf5d70300020009a326e203000200\
         b3f7e64703000200f41d1cdc0300020003c5580803000200\
         42ee131d030002001e80eda7030002003200"
    );
    assert_eq!(Layout::da_assinatura().total, 163);
}

/// Nenhum pedido sai pelo reset de nó, nem para o nó errado.
#[test]
fn nenhum_pedido_e_o_reset_de_no() {
    for p in [
        Pedido::RegistrarNo,
        Pedido::Cancelar { mensagem: 1 },
        Pedido::Assinar {
            mensagem: 1,
            dados: &ASSINATURA,
            frequencia: 50,
        },
        Pedido::Responder { seq: 9 },
    ] {
        let q = p.quadro(1);
        assert_ne!((q.cmd_set, q.cmd_id), (0x48, 0x02), "{p:?}");
        assert_eq!((q.origem, q.destino), (0x09, 0x03), "{p:?}");
    }
}

/// A ordem em que a bateria vem depois das rodas: 2 de 6 gravações.
fn layout_outra_ordem() -> Layout {
    Layout::de(&[
        TORRE_BASICO,
        VELOCIDADE,
        RODAS,
        BATERIA,
        POSICAO,
        barramento::IMU,
        barramento::ESTADO,
        barramento::ATITUDE,
        barramento::SERVO,
    ])
    .unwrap()
}

/// Pacote gravado do robô, na ordem com a bateria na posição 84.
#[test]
fn o_pacote_gravado_cai_nas_faixas() {
    for gravado in [PARADO, ANDANDO] {
        let q = Quadro::ler(gravado).unwrap();
        assert_eq!((q.cmd_set, q.cmd_id), id::PACOTE);
        let p = Pacote::ler(&q.dados, &layout_outra_ordem()).unwrap();
        assert!((9000..13500).contains(&p.bateria.milivolts), "{p:?}");
        assert!(p.bateria.percentual <= 100, "{p:?}");
        assert!((-1.2..-0.8).contains(&p.aceleracao[2]), "{p:?}");
        assert!(p.estado <= 2047, "{p:?}");
    }
}

#[test]
fn tamanho_errado_e_erro_e_nao_valor_falso() {
    let q = Quadro::ler(PARADO).unwrap();
    let e = Pacote::ler(&q.dados[..150], &Layout::da_assinatura()).unwrap_err();
    assert_eq!((e.tem, e.precisa), (150, 163));
}

/// O único golpe gravado: `58` para `09`, `20 23 01 22 00`.
#[test]
fn o_golpe_gravado_e_a_placa_2() {
    let g = barramento::ler_golpe(&bytes("2023012200")).unwrap();
    assert_eq!(
        (g.placa, g.tipo, g.microfone, g.aceleracao),
        (2, 0, 0x0123, 0x0022)
    );
    assert!(barramento::ler_golpe(&[1, 2]).is_err());
}

#[test]
fn torre_e_estado_tem_as_posicoes_medidas() {
    let t = barramento::ler_torre_posicao(&[1, 0, 2, 0, 3, 0, 4, 0, 0]).unwrap();
    assert_eq!(
        (t.rumo_solo, t.inclinacao_solo, t.rumo, t.inclinacao),
        (1, 2, 3, 4)
    );
    assert_eq!(
        barramento::ler_estado_chassi(&[0x20, 0, 0, 0]).unwrap(),
        0x20
    );
    assert_eq!(barramento::ler_infravermelho(&[0, 0xf2, 0]).unwrap(), 2);
    assert!(barramento::ler_torre_posicao(&[0; 8]).is_err());
}

#[test]
fn cada_codigo_da_tabela_tem_sentido() {
    assert!(barramento::sentido_do_codigo(0x23).contains("já está assinado"));
    assert!(barramento::sentido_do_codigo(0x0f).contains("não existe"));
    assert_eq!(barramento::sentido_do_codigo(0x7f), "código desconhecido");
}
