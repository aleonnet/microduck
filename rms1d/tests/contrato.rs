//! Toda chamada do contrato tem resposta decidida.

use std::sync::Arc;

use duck_ipc_proto as proto;
use rms1_control::{Intencoes, Saude};
use rms1d::respostas::{Contexto, olhar, responder};
use rms1d_params::Params;

fn contexto() -> Contexto {
    Contexto {
        params: Params::default(),
        intencoes: Arc::new(Intencoes::default()),
        saude: Arc::new(Saude::default()),
    }
}

/// Chamada sem resposta deixaria o cliente pendurado. Todas respondem, e com o id.
#[test]
fn toda_chamada_do_contrato_responde() {
    let c = contexto();
    for call in proto::test_support::every_call() {
        let r = responder(&c, proto::Id::Number(7), &call);
        assert_eq!(r.id, Some(proto::Id::Number(7)), "{}", call.method());
    }
}

/// Os modos passam por `robot.do`, porque o console recusa `s1.*` (`mediad/src/session.rs`).
#[test]
fn skill_de_modo_chega_as_intencoes() {
    let c = contexto();
    let r = responder(
        &c,
        proto::Id::Number(1),
        &proto::Call::RobotDo(proto::DoParams {
            skill: "torre_segue_chassi".to_owned(),
        }),
    );
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["result"]["accepted"], true);
    assert_eq!(c.intencoes.modo(), rms1_control::Modo::TorreSegueChassi);
}

fn look(x: f64, y: f64, z: f64) -> proto::LookResult {
    olhar(
        &proto::LookParams {
            x,
            y,
            z,
            neck_pitch: 0.0,
        },
        &Params::default().seguranca,
    )
}

/// À esquerda é yaw positivo; acima é pitch negativo. Convenção do contrato.
#[test]
fn olhar_para_um_ponto_da_os_sinais_do_contrato() {
    let r = look(1.0, 1.0, 0.0);
    assert!((r.head.head_yaw - std::f64::consts::FRAC_PI_4).abs() < 1e-9);
    assert_eq!(r.head.head_pitch, 0.0);
    assert!(!r.clamped);
    let r = look(1.0, 0.0, 0.2);
    assert!(r.head.head_pitch < 0.0, "acima é pitch negativo");
    assert!(!r.clamped);
}

/// Limites do firmware: 35° para cima e 20° para baixo.
#[test]
fn olhar_alem_do_alcance_e_limitado_e_diz_isso() {
    let r = look(0.1, 0.0, 10.0);
    assert!(r.clamped);
    assert!((r.head.head_pitch + 35f64.to_radians()).abs() < 1e-9);
    let r = look(0.1, 0.0, -10.0);
    assert!(r.clamped);
    assert!((r.head.head_pitch - 20f64.to_radians()).abs() < 1e-9);
}

#[test]
fn a_origem_nao_tem_direcao_e_olha_em_frente() {
    let r = look(0.0, 0.0, 0.0);
    assert_eq!((r.head.head_yaw, r.head.head_pitch), (0.0, 0.0));
}
