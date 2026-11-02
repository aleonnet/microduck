//! Cada comando contra a biblioteca e contra o que a DJI ou o robô fizeram.

use rms1_proto::comando::{EIXO_PITCH, EIXO_YAW, byte_do_no};
use rms1_proto::{Comando, Nos, PermissaoForma, Quadro};

const BATIMENTO_DJI: &[u8] = include_bytes!("../../s1-duml/tests/fixtures/heartbeat_real.bin");
const BATIMENTO_PROROKLAB: &[u8] = include_bytes!("../../s1-duml/tests/fixtures/poc_hb.bin");
const RODAS_FRENTE: &[u8] = include_bytes!("../../s1-duml/tests/fixtures/poc_fwd.bin");
const TORRE: &[u8] = include_bytes!("../../s1-duml/tests/fixtures/poc_gim.bin");

/// Dados e destino iguais ao quadro gravado; a sequência é do chamador.
fn igual_ao_gravado(c: Comando, gravado: &[u8]) {
    let g = Quadro::ler(gravado).unwrap();
    let q = c.quadro(&Nos::default(), g.seq);
    assert_eq!(q.montar().unwrap(), gravado, "{c:?}");
}

#[test]
fn numero_da_dji_vira_o_byte_do_quadro() {
    assert_eq!(byte_do_no(306), 0xC3);
    assert_eq!(byte_do_no(906), 0xC9);
    assert_eq!(byte_do_no(900), 0x09);
    assert_eq!(byte_do_no(400), 0x04);
    assert_eq!(byte_do_no(300), 0x03);
}

#[test]
fn permissao_da_dji_e_o_quadro_gravado_do_robo() {
    igual_ao_gravado(Comando::Permissao(PermissaoForma::Dji), BATIMENTO_DJI);
}

#[test]
fn permissao_plano_b_e_a_que_moveu_o_robo() {
    igual_ao_gravado(
        Comando::Permissao(PermissaoForma::Proroklab),
        BATIMENTO_PROROKLAB,
    );
}

/// As quatro rodas para a frente, 40 rpm: a roda 2 e a 3 vão invertidas, como `rm_ctrl.py:394`, e é
/// o quadro que moveu o robô.
#[test]
fn rodas_para_a_frente_sao_o_quadro_que_moveu_o_robo() {
    igual_ao_gravado(Comando::Rodas([40, 40, 40, 40]), RODAS_FRENTE);
}

#[test]
fn rodas_tem_posicao_e_tipo_da_biblioteca() {
    let q = Comando::Rodas([100, -200, 300, -400]).quadro(&Nos::default(), 0);
    assert_eq!((q.cmd_set, q.cmd_id), (0x3f, 0x20));
    assert_eq!((q.origem, q.destino), (0xC9, 0xC3));
    let i = |p: usize| i16::from_le_bytes([q.dados[p], q.dados[p + 1]]);
    assert_eq!(q.dados.len(), 8);
    assert_eq!((i(0), i(2), i(4), i(6)), (100, 200, -300, -400));
}

/// O quadro que moveu a torre em 2026-10-29 usa coordenada 3; a forma é a mesma, e a coordenada é
/// parâmetro.
#[test]
fn torre_com_coordenada_antiga_e_o_quadro_que_moveu_o_robo() {
    igual_ao_gravado(
        Comando::TorreAngulos {
            yaw_ddeg: 150,
            pitch_ddeg: 100,
            eixos: EIXO_YAW | EIXO_PITCH,
            coordenada: 3,
            velocidade: 30,
        },
        TORRE,
    );
}

#[test]
fn torre_tem_posicoes_da_biblioteca() {
    let q = Comando::TorreAngulos {
        yaw_ddeg: -1234,
        pitch_ddeg: 200,
        eixos: EIXO_PITCH,
        coordenada: 4,
        velocidade: 90,
    }
    .quadro(&Nos::default(), 0);
    assert_eq!((q.cmd_set, q.cmd_id), (0x3f, 0xb0));
    assert_eq!(q.destino, 0x04);
    assert_eq!(q.dados.len(), 17);
    assert_eq!(q.dados[2], (4 << 3) | 0x04);
    let i = |p: usize| i16::from_le_bytes([q.dados[p], q.dados[p + 1]]);
    assert_eq!((i(3), i(5), i(7)), (-1234, 0, 200));
    assert_eq!((i(11), i(15)), (90, 90));
}

#[test]
fn seguir_tem_tres_floats_em_ordem() {
    let q = Comando::Seguir {
        vx: 1.5,
        vy: -0.25,
        angulo: 12.0,
    }
    .quadro(&Nos::default(), 0);
    assert_eq!((q.cmd_set, q.cmd_id), (0x3f, 0x22));
    let f = |p: usize| f32::from_le_bytes(q.dados[p..p + 4].try_into().unwrap());
    assert_eq!((f(0), f(4), f(8)), (1.5, -0.25, 12.0));
}

#[test]
fn modos_tem_codigo_e_destino_da_biblioteca() {
    let n = Nos::default();
    let m = Comando::ModoChassi(1).quadro(&n, 0);
    assert_eq!(
        ((m.cmd_set, m.cmd_id), m.destino, m.dados),
        ((0x3f, 0x19), 0xC3, vec![1])
    );
    let t = Comando::ModoTorre(2).quadro(&n, 0);
    assert_eq!(
        ((t.cmd_set, t.cmd_id), t.destino, t.dados),
        ((0x04, 0x4c), 0x04, vec![2, 0])
    );
    let s = Comando::Sobrepor(0).quadro(&n, 0);
    assert_eq!(((s.cmd_set, s.cmd_id), s.dados), ((0x3f, 0x28), vec![0]));
}

/// O reset de nó deixou o barramento do robô morto até desligar na chave, e parar `dji_scratch` derrubou
/// `dji_sys`. Nenhum dos dois pode existir no código novo.
#[test]
fn o_codigo_novo_nao_tem_reset_de_no_nem_para_dji_scratch() {
    let raiz = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
    let mut achados = Vec::new();
    for pasta in [
        "rms1-proto/src",
        "rms1-control/src",
        "rms1d/src",
        "rms1d-params/src",
    ] {
        let Ok(arquivos) = std::fs::read_dir(format!("{raiz}/{pasta}")) else {
            continue;
        };
        for a in arquivos.flatten() {
            let texto = std::fs::read_to_string(a.path()).unwrap_or_default();
            let sem_espaco = texto.replace(' ', "").to_lowercase();
            if sem_espaco.contains("(0x48,0x02)") || sem_espaco.contains("0x48/0x02") {
                achados.push(format!("{:?}: reset de nó", a.path()));
            }
            if texto.contains("stop dji_scratch") {
                achados.push(format!("{:?}: para dji_scratch", a.path()));
            }
        }
    }
    let toml = std::fs::read_to_string(format!("{raiz}/deploy/s1/rms1d.toml")).unwrap_or_default();
    let ativas: String = toml
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect();
    if ativas.contains("dji_scratch") {
        achados.push("deploy/s1/rms1d.toml: dji_scratch na lista de processos".to_owned());
    }
    assert!(achados.is_empty(), "{achados:?}");
}
