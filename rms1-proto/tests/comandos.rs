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
            tarefa: 0,
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
        tarefa: 7,
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
    assert_eq!(q.dados[0], 7, "o identificador da tarefa");
    assert_eq!(q.dados[2], (4 << 3) | 0x04);
    let i = |p: usize| i16::from_le_bytes([q.dados[p], q.dados[p + 1]]);
    assert_eq!((i(3), i(5), i(7)), (-1234, 0, 200));
    assert_eq!((i(11), i(15)), (90, 90));
}

#[test]
fn taxa_da_torre_tem_posicoes_da_biblioteca() {
    let q = Comando::TorreTaxa {
        yaw_ddeg_s: -300,
        pitch_ddeg_s: 50,
        controle: rms1_proto::comando::TAXA_MOVENDO,
    }
    .quadro(&Nos::default(), 0);
    assert_eq!((q.cmd_set, q.cmd_id, q.destino), (0x04, 0x0c, 0x04));
    let i = |p: usize| i16::from_le_bytes([q.dados[p], q.dados[p + 1]]);
    assert_eq!((i(0), i(2), i(4), q.dados[6]), (-300, 0, 50, 0xdc));
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

/// Todo comando que pode sair, um de cada: se alguém criar uma variante nova, o `match` em `id()` obriga a
/// dizer o código dela, e este teste precisa dela aqui para continuar valendo.
fn um_de_cada() -> Vec<Comando> {
    vec![
        Comando::Permissao(PermissaoForma::Dji),
        Comando::Permissao(PermissaoForma::Proroklab),
        Comando::ModoChassi(1),
        Comando::Rodas([0; 4]),
        Comando::Seguir {
            vx: 0.0,
            vy: 0.0,
            angulo: 0.0,
        },
        Comando::Sobrepor(0),
        Comando::ModoTorre(0),
        Comando::TorreAngulos {
            tarefa: 1,
            yaw_ddeg: 0,
            pitch_ddeg: 0,
            eixos: EIXO_YAW,
            coordenada: 4,
            velocidade: 30,
        },
        Comando::TorreTaxa {
            yaw_ddeg_s: 0,
            pitch_ddeg_s: 0,
            controle: 0,
        },
        rms1_proto::comando::sensibilidade_das_placas(5),
    ]
}

/// O reset de nó é do conjunto 0x48: nenhum comando sai por ele.
#[test]
fn nenhum_comando_e_do_conjunto_do_reset_de_no() {
    for c in um_de_cada() {
        assert_ne!(c.id().0, 0x48, "{c:?}");
        assert_ne!(c.quadro(&Nos::default(), 0).cmd_set, 0x48, "{c:?}");
    }
}

fn varrer(caminho: &std::path::Path, arquivos: &mut Vec<std::path::PathBuf>) {
    if caminho.is_dir() {
        for a in std::fs::read_dir(caminho).unwrap().flatten() {
            varrer(&a.path(), arquivos);
        }
    } else {
        arquivos.push(caminho.to_owned());
    }
}

/// Por texto, de leve: o reset de nó, do conjunto da telemetria, deixou o barramento morto até desligar, e parar `dji_scratch` derrubou
/// `dji_sys`. Pega quem escreve o código do reset ou manda parar `dji_scratch`, em qualquer pasta do serviço
/// e nos scripts dele. A garantia de verdade é o teste acima, mais `Seguranca` ser a única dona do corpo.
#[test]
fn o_codigo_novo_nao_cita_reset_de_no_nem_para_dji_scratch() {
    let raiz = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let mut arquivos = Vec::new();
    for pasta in [
        "rms1-proto/src",
        "rms1-control/src",
        "rms1d/src",
        "rms1d-params/src",
    ] {
        varrer(&raiz.join(pasta), &mut arquivos);
    }
    for script in [
        "scripts/rms1",
        "scripts/test-rms1d-sim.sh",
        "scripts/s1/rms1d-bench.sh",
        "scripts/s1/rms1d-ponte.py",
    ] {
        arquivos.push(raiz.join(script));
    }
    assert!(arquivos.len() > 15, "{arquivos:?}");
    let mut achados = Vec::new();
    for a in arquivos {
        let texto = std::fs::read_to_string(&a).unwrap();
        let junto = texto.replace(' ', "").to_lowercase();
        // O conjunto em si passou a existir, com a telemetria: o proibido é o comando 2 dele, escrito de qualquer jeito.
        if [
            "0x48,0x02",
            "0x48,2",
            "0x48/0x02",
            "0x48/2",
            "72,2",
            "72/2",
            "72,0x02",
            "0x4802",
        ]
        .iter()
        .any(|f| junto.contains(f))
        {
            achados.push(format!("{}: reset de nó", a.display()));
        }
        if texto.contains("stop dji_scratch") || texto.contains("ctl.stop dji_scratch") {
            achados.push(format!("{}: para dji_scratch", a.display()));
        }
    }
    let toml = std::fs::read_to_string(raiz.join("deploy/s1/rms1d.toml")).unwrap();
    let ativas: String = toml
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect();
    if ativas.contains("dji_scratch") {
        achados.push("deploy/s1/rms1d.toml: dji_scratch na lista de processos".to_owned());
    }
    assert!(achados.is_empty(), "{achados:?}");
}

/// O quadro que a central da DJI mandou à placa 2401, gravado duas vezes na bancada de 150 s (`hdvt_uav.strace`, no
/// laboratório, às 17:07:22 e 17:07:28 de 2026-10-30), com a sensibilidade padrão, nível 5.
const ARMADURA_VOZ_NIVEL_5: [u8; 32] = [
    0x55, 0x20, 0x04, 0x7b, 0x09, 0x38, 0x03, 0x00, 0x40, 0x3f, 0x07, 0x00, 0xf4, 0x01, 0x2c, 0x01,
    0x32, 0x00, 0x0d, 0x00, 0x06, 0x00, 0x01, 0x00, 0xa0, 0x00, 0xb4, 0x00, 0xc8, 0x00, 0x92, 0xda,
];

#[test]
fn sensibilidade_padrao_e_o_quadro_gravado_da_central() {
    igual_ao_gravado(
        rms1_proto::comando::sensibilidade_das_placas(5),
        &ARMADURA_VOZ_NIVEL_5,
    );
}

/// Os picos dos 11 níveis, pela conta da DJI rodada em Python (`int(160 * (1.5 - nivel / 10))`, e 180 e 200): o truncamento
/// dá 251 no nível 1 e 125 no nível 8.
#[test]
fn os_picos_de_cada_nivel_sao_os_da_conta_da_dji() {
    let esperado: [[u16; 3]; 11] = [
        [240, 270, 300],
        [224, 251, 280],
        [208, 234, 260],
        [192, 216, 240],
        [176, 198, 220],
        [160, 180, 200],
        [144, 162, 180],
        [128, 144, 160],
        [112, 125, 140],
        [96, 108, 120],
        [80, 90, 100],
    ];
    for (nivel, e) in esperado.iter().enumerate() {
        assert_eq!(
            rms1_proto::comando::sensibilidade_das_placas(nivel as u8),
            Comando::ArmaduraVoz { picos: *e },
            "nível {nivel}"
        );
    }
    assert_eq!(
        rms1_proto::comando::sensibilidade_das_placas(200),
        Comando::ArmaduraVoz {
            picos: esperado[10]
        },
        "acima de 10, o 10, como a DJI limita"
    );
}
