//! Toda chamada do contrato tem resposta decidida.

use std::sync::Arc;

use duck_ipc_proto as proto;
use rms1_control::{Intencoes, Saude};
use rms1d::respostas::{
    Contexto, aplicar_taxa_da_torre, olhar, responder, saude_do_contrato, sensibilidade_das_placas,
};
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

/// Estas duas respondem um documento, e não `accepted`: a recusa viraria um caderno vazio em `robotctl vetbook`.
/// Toda chamada que não é de intenção precisa de erro, ou o cliente acredita no vazio.
#[test]
fn o_que_responde_documento_e_erro_e_nao_recusa() {
    let c = contexto();
    for call in [proto::Call::RobotVetbook, proto::Call::RobotModel] {
        let r = responder(&c, proto::Id::Number(1), &call);
        let v = serde_json::to_value(&r).unwrap();
        assert!(v["result"].is_null(), "{}: {v}", call.method());
        assert_eq!(v["error"]["code"], proto::code::METHOD_NOT_FOUND, "{v}");
    }
}

/// O prazo é o de `rms1d.toml`, e não um número no código, nos dois sentidos.
#[test]
fn seguro_para_reiniciar_usa_o_prazo_do_arquivo() {
    let seguro = |prazo_ms| {
        let mut c = contexto();
        c.params.seguranca.prazo_ms = prazo_ms;
        c.intencoes.pedir_armar(true);
        c.intencoes.pedir_twist([0.5, 0.0, 0.0]);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let r = responder(&c, proto::Id::Number(1), &proto::Call::RobotSafeToRestart);
        serde_json::to_value(&r).unwrap()["result"]["safe"].clone()
    };
    assert_eq!(seguro(3000), false, "ainda dentro do prazo");
    assert_eq!(seguro(100), true, "passou do prazo");
}

/// No robô, o controlador fala o tempo todo: serial sem nada chegando é fio solto ou processo errado, e `healthy` mentiria.
#[test]
fn serial_muda_nao_e_saudavel() {
    let mut c = contexto();
    c.params.corpo.tipo = rms1d_params::TipoCorpo::Serial;
    c.params.laco.serial_muda_ms = 100;
    let agora = || std::time::Instant::now();
    c.saude.carimbar_ciclo(agora());
    assert!(saude_do_contrato(&c).healthy, "logo ao subir ainda é cedo");
    std::thread::sleep(std::time::Duration::from_millis(150));
    c.saude.carimbar_ciclo(agora());
    let h = saude_do_contrato(&c);
    assert!(!h.healthy && h.degraded, "{h:?}");
    assert!(h.reason.unwrap().contains("mudo"));
    // Chegou um quadro: acabou o problema.
    c.saude
        .quadros_recebidos
        .store(1, std::sync::atomic::Ordering::Relaxed);
    assert!(saude_do_contrato(&c).healthy);
    // No simulador sem página nada volta, e isso não é defeito.
    c.params.corpo.tipo = rms1d_params::TipoCorpo::Rede;
    c.saude
        .quadros_recebidos
        .store(0, std::sync::atomic::Ordering::Relaxed);
    assert!(saude_do_contrato(&c).healthy);
}

fn com_amostra(c: &Contexto, enlace: rms1_control::Enlace) {
    use rms1_control::telemetria::{BITS_CHASSI, PLACAS, Placa};
    c.saude
        .publicar_amostra(std::sync::Arc::new(rms1_control::Amostra {
            enlace,
            pacotes: 1,
            ignorados: 0,
            medidas: None,
            torre: None,
            estado_chassi: 0,
            contagem_chassi: [0; BITS_CHASSI.len()],
            placas: [Placa::default(); PLACAS],
            infravermelho: [0; 2],
            estimativa: None,
        }));
}

/// Pacote velho não pode parecer vivo: a saúde diz, com a palavra que a bancada procura.
#[test]
fn telemetria_parada_nao_e_saudavel() {
    let mut c = contexto();
    c.params.corpo.tipo = rms1d_params::TipoCorpo::Rede;
    c.params.telemetria.idade_maxima_ms = 50;
    c.saude.carimbar_ciclo(std::time::Instant::now());
    com_amostra(&c, rms1_control::Enlace::Recebendo);
    c.saude.carimbar_pacote(std::time::Instant::now());
    assert!(saude_do_contrato(&c).healthy, "pacote recente");
    std::thread::sleep(std::time::Duration::from_millis(80));
    c.saude.carimbar_ciclo(std::time::Instant::now());
    let h = saude_do_contrato(&c);
    assert!(!h.healthy && h.degraded, "{h:?}");
    assert!(h.reason.unwrap().contains("telemetria parada"));
}

#[test]
fn recusa_diz_o_codigo_e_o_sentido() {
    let mut c = contexto();
    c.params.corpo.tipo = rms1d_params::TipoCorpo::Serial;
    c.saude
        .quadros_recebidos
        .store(1, std::sync::atomic::Ordering::Relaxed);
    c.saude.carimbar_ciclo(std::time::Instant::now());
    com_amostra(
        &c,
        rms1_control::Enlace::Recusado {
            passo: rms1_control::telemetria::Passo::Assinar,
            codigo: 0x23,
        },
    );
    let h = saude_do_contrato(&c);
    assert!(!h.healthy && h.degraded, "{h:?}");
    let m = h.reason.unwrap();
    assert!(
        m.contains("telemetria") && m.contains("0x23") && m.contains("já está assinado"),
        "{m}"
    );
}

/// O simulador sem página, com o envio falhando: nem doente, nem corpo que não recebe.
#[test]
fn simulador_sem_pagina_segue_saudavel() {
    let mut c = contexto();
    c.params.corpo.tipo = rms1d_params::TipoCorpo::Rede;
    c.saude.carimbar_ciclo(std::time::Instant::now());
    com_amostra(
        &c,
        rms1_control::Enlace::SemResposta {
            passo: rms1_control::telemetria::Passo::Registrar,
        },
    );
    c.saude
        .erros_barramento
        .store(50, std::sync::atomic::Ordering::Relaxed);
    let h = saude_do_contrato(&c);
    assert!(h.healthy, "{h:?}");
    assert_eq!(h.bus.consecutive_errors, 0);
}

/// O que nunca chegou é ausente, e não zero: bateria zero é outra coisa.
#[test]
fn sem_pacote_nao_ha_bateria_nem_corpo() {
    let c = contexto();
    assert!(saude_do_contrato(&c).battery.is_none());
    let v = rms1d::respostas::estado_s1(None, None, false);
    assert_eq!(v["link"]["state"], "off");
    assert!(v["body"].is_null());
}

/// Critério 21: o nosso cálculo sai em `s1.state.estimate`, ao lado do `body`, e o `robot.state` segue com o controlador.
#[test]
fn o_calculo_proprio_sai_em_estimate_e_nao_muda_o_robot_state() {
    use rms1_control::estimativa::Estimativa;
    use rms1_control::telemetria::{BITS_CHASSI, Medidas, PLACAS, Placa};
    let p = Params::default();
    let m = Medidas {
        velocidade_solo: [0.0; 3],
        velocidade_corpo: [2.5, 0.0, 0.0],
        rodas_rpm: [0.0; 4],
        rodas_angulo_grau: [0.0; 4],
        rodas_estado: [0; 4],
        posicao: [0.3, 0.4, 0.0],
        aceleracao: [0.0, 0.0, 1.0],
        giro: [0.0; 3],
        rumo: 10.0,
        inclinacao: 0.0,
        rolagem: 0.0,
        volts: 12.0,
        percentual: 90.0,
        corrente_a: 0.0,
        temperatura_c: 30.0,
        relogio_ms: 0,
    };
    let mut e = Estimativa::new(&p.estimativa, &p.geometria);
    let mut calc = e.passo(&m, 0);
    // 3 s de pacotes, acima do tempo do levantado.
    for i in 1..150 {
        calc = e.passo(
            &Medidas {
                relogio_ms: 20 * i,
                ..m
            },
            0,
        );
    }
    assert!(
        calc.levantado,
        "o controlador acusa 2,5 m/s e as rodas não andam"
    );
    let amostra = rms1_control::Amostra {
        enlace: rms1_control::Enlace::Recebendo,
        pacotes: 150,
        ignorados: 0,
        medidas: Some(m),
        torre: None,
        estado_chassi: 0,
        contagem_chassi: [0; BITS_CHASSI.len()],
        placas: [Placa::default(); PLACAS],
        infravermelho: [0; 2],
        estimativa: Some(calc),
    };
    let s = rms1d::respostas::estado_s1(Some(&amostra), Some(10), true);
    assert_eq!(s["estimate"]["picked_up"], true);
    assert_eq!(
        s["estimate"]["velocity_body"],
        serde_json::json!([0.0, 0.0, 0.0])
    );
    assert_eq!(
        s["body"]["velocity_body"],
        serde_json::json!([2.5, 0.0, 0.0]),
        "o do controlador fica"
    );
    assert_eq!(s["body"]["position_m"], serde_json::json!([0.3, 0.4, 0.0]));
    let estado = rms1_control::Estado {
        t: 1.0,
        pedido: [0.0; 3],
        aplicado: [0.0; 3],
        limitado_por: Vec::new(),
        torre: None,
        modo: rms1_control::Modo::Livre,
        armado: false,
        hz: 100.0,
        perdidos: 0,
        corpo: Some(Arc::new(amostra)),
        corpo_recente: true,
        idade_pacote_ms: Some(10),
    };
    let r = rms1d::respostas::estado_do_contrato(&estado);
    assert!(
        !r.safety.picked_up,
        "o robot.state segue o bit do controlador neste plano"
    );
    assert_eq!(
        r.odom.position,
        [0.3, 0.4, 0.0],
        "e a posição do controlador"
    );
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

/// `s1.gimbal`: só número vale, e a ação tem de ser conhecida.
#[test]
fn velocidade_da_torre_so_aceita_o_previsto() {
    let c = contexto();
    let pede = |v: serde_json::Value| aplicar_taxa_da_torre(&c.intencoes, Some(&v));
    assert!(pede(serde_json::json!({"action": "speed", "yaw": 30.0, "pitch": -10.0})).is_ok());
    assert!(pede(serde_json::json!({"action": "stop"})).is_ok());
    assert!(pede(serde_json::json!({"action": "speed", "yaw": "muito", "pitch": 0.0})).is_err());
    assert!(pede(serde_json::json!({"action": "speed", "yaw": 30.0})).is_err());
    assert!(pede(serde_json::json!({"action": "girar"})).is_err());
    assert!(aplicar_taxa_da_torre(&c.intencoes, None).is_err());
}

/// `s1.armor`: nível inteiro de 0 a 10, a faixa da DJI; sem nível, responde o atual, o do arquivo até alguém pedir.
#[test]
fn sensibilidade_das_placas_so_aceita_de_0_a_10_e_responde_a_atual() {
    let c = contexto();
    let pede = |v: Option<serde_json::Value>| sensibilidade_das_placas(&c, v.as_ref());
    assert_eq!(
        pede(None).unwrap()["sensitivity"],
        c.params.armadura.sensibilidade
    );
    assert_eq!(
        pede(Some(serde_json::json!({"sensitivity": 4}))).unwrap()["sensitivity"],
        4
    );
    assert_eq!(c.intencoes.tomar_sensibilidade(), Some(4));
    assert_eq!(pede(Some(serde_json::json!({}))).unwrap()["sensitivity"], 4);
    for ruim in [
        serde_json::json!({"sensitivity": 11}),
        serde_json::json!({"sensitivity": -1}),
        serde_json::json!({"sensitivity": 2.5}),
        serde_json::json!({"sensitivity": "alto"}),
    ] {
        assert!(pede(Some(ruim.clone())).is_err(), "{ruim}");
    }
    assert_eq!(
        c.intencoes.tomar_sensibilidade(),
        None,
        "recusado não pede nada"
    );
}
