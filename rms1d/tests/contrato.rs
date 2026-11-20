//! Toda chamada do contrato tem resposta decidida.

use std::sync::Arc;

use duck_ipc_proto as proto;
use rms1_control::{FonteId, Intencoes, Saude};
use rms1d::respostas::{
    Contexto, Controle, aplicar_taxa_da_torre, mira, olhar, responder, saude_do_contrato,
    sensibilidade_das_placas,
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
        let r = responder(&c, FonteId::SEM_NOME, proto::Id::Number(7), &call);
        assert_eq!(r.id, Some(proto::Id::Number(7)), "{}", call.method());
    }
}

/// Estas respondem um documento, e não `accepted`: a recusa viraria um caderno vazio em `robotctl vetbook`, e um volume 0
/// que ninguém pôs. Toda chamada que não é de intenção precisa de erro, ou o cliente acredita no vazio.
#[test]
fn o_que_responde_documento_e_erro_e_nao_recusa() {
    let c = contexto();
    for call in [
        proto::Call::RobotVetbook,
        proto::Call::RobotModel,
        proto::Call::RobotVolume,
        proto::Call::RobotSetVolume(proto::VolumeParams { volume: 40 }),
    ] {
        let r = responder(&c, FonteId::SEM_NOME, proto::Id::Number(1), &call);
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
        c.intencoes.pedir_twist(FonteId::SEM_NOME, [0.5, 0.0, 0.0]);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let r = responder(
            &c,
            FonteId::SEM_NOME,
            proto::Id::Number(1),
            &proto::Call::RobotSafeToRestart,
        );
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
    let v = rms1d::respostas::estado_s1(None, None, false, &Controle::default());
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
    let s = rms1d::respostas::estado_s1(Some(&amostra), Some(10), true, &Controle::default());
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
        dono_rodas: None,
        dono_torre: None,
        fonte_do_pedido: None,
        autonomia: false,
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
        FonteId::SEM_NOME,
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
    let pede = |v: serde_json::Value| {
        aplicar_taxa_da_torre(&c.intencoes, FonteId::SEM_NOME, Some(&v), true)
    };
    assert!(pede(serde_json::json!({"action": "speed", "yaw": 30.0, "pitch": -10.0})).is_ok());
    assert!(pede(serde_json::json!({"action": "stop"})).is_ok());
    assert!(pede(serde_json::json!({"action": "speed", "yaw": "muito", "pitch": 0.0})).is_err());
    assert!(pede(serde_json::json!({"action": "speed", "yaw": 30.0})).is_err());
    assert!(pede(serde_json::json!({"action": "girar"})).is_err());
    assert!(aplicar_taxa_da_torre(&c.intencoes, FonteId::SEM_NOME, None, true).is_err());
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

/// O começo de `/data/rm_user_setting.json` do robô do dono, na forma em que o `dji_hdvt_uav` o escreve (cópia de 2026-11-01,
/// `out/2026-11-01-copia-completa/arvores/data_soltos/rm_user_setting.json`, no laboratório), com as chaves que vêm depois.
const AJUSTES_DA_DJI: &str = "{\n   \"front_sight_zoom_inx\" : 5000,\n   \"front_sight_zoom_iny\" : 5000,\n   \"front_sight_zoom_outx\" : 5000,\n   \"front_sight_zoom_outy\" : 5000,\n   \"func_enable\" : {\n      \"game_scene\" : [ true, true, true, true, true, false, true ]\n   },\n   \"goback_enable\" : 0,\n   \"low_power_time\" : 60,\n   \"speaker_volume\" : 15\n}\n";

fn contexto_com_mira(nome: &str, conteudo: Option<&str>) -> (Contexto, std::path::PathBuf) {
    let pasta = std::env::temp_dir().join(format!("rms1d-mira-{}-{nome}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pasta);
    std::fs::create_dir_all(&pasta).unwrap();
    let arquivo = pasta.join("rm_user_setting.json");
    if let Some(c) = conteudo {
        std::fs::write(&arquivo, c).unwrap();
    }
    let mut c = contexto();
    c.params.mira.arquivo = Some(arquivo.to_string_lossy().into_owned());
    (c, arquivo)
}

/// `s1.sight` sem valores: as quatro do arquivo; sem `[mira] arquivo`, sem o arquivo ou sem a chave, o padrão de fábrica, 5000.
#[test]
fn a_mira_le_o_arquivo_da_dji_e_o_padrao_e_o_centro() {
    let r = mira(&contexto(), None).unwrap();
    assert_eq!(
        r,
        serde_json::json!({"zoom_in": [5000, 5000], "zoom_out": [5000, 5000]})
    );
    let (c, _) = contexto_com_mira("ausente", None);
    assert_eq!(
        mira(&c, None).unwrap()["zoom_out"],
        serde_json::json!([5000, 5000])
    );
    let (c, _) = contexto_com_mira(
        "le",
        Some(&AJUSTES_DA_DJI.replace(
            "\"front_sight_zoom_outy\" : 5000",
            "\"front_sight_zoom_outy\" : 6200",
        )),
    );
    assert_eq!(
        mira(&c, None).unwrap(),
        serde_json::json!({"zoom_in": [5000, 5000], "zoom_out": [5000, 6200]})
    );
    let (c, _) = contexto_com_mira("sem-chave", Some("{\n   \"speaker_volume\" : 15\n}\n"));
    assert_eq!(
        mira(&c, Some(&serde_json::json!({}))).unwrap()["zoom_in"],
        serde_json::json!([5000, 5000])
    );
}

/// Gravar muda só os quatro números: o resto do arquivo da DJI fica byte a byte, e a leitura devolve o gravado.
#[test]
fn gravar_a_mira_muda_so_os_quatro_numeros() {
    let (c, arquivo) = contexto_com_mira("grava", Some(AJUSTES_DA_DJI));
    let r = mira(
        &c,
        Some(&serde_json::json!({"zoom_in": [1234, 4321], "zoom_out": [600, 9000]})),
    )
    .unwrap();
    assert_eq!(
        r,
        serde_json::json!({"zoom_in": [1234, 4321], "zoom_out": [600, 9000]})
    );
    let esperado = AJUSTES_DA_DJI
        .replace(
            "\"front_sight_zoom_inx\" : 5000",
            "\"front_sight_zoom_inx\" : 1234",
        )
        .replace(
            "\"front_sight_zoom_iny\" : 5000",
            "\"front_sight_zoom_iny\" : 4321",
        )
        .replace(
            "\"front_sight_zoom_outx\" : 5000",
            "\"front_sight_zoom_outx\" : 600",
        )
        .replace(
            "\"front_sight_zoom_outy\" : 5000",
            "\"front_sight_zoom_outy\" : 9000",
        );
    assert_eq!(std::fs::read_to_string(&arquivo).unwrap(), esperado);
    // O padrão volta ao centro, e o arquivo volta ao que era.
    mira(
        &c,
        Some(&serde_json::json!({"zoom_in": [5000, 5000], "zoom_out": [5000, 5000]})),
    )
    .unwrap();
    assert_eq!(std::fs::read_to_string(&arquivo).unwrap(), AJUSTES_DA_DJI);
    assert!(
        !arquivo.with_extension("json.rms1d").exists(),
        "o temporário não fica"
    );
}

/// Chave ausente entra no arquivo, e o resto fica; num objeto vazio, o arquivo continua válido.
#[test]
fn gravar_a_mira_sem_as_chaves_as_acrescenta() {
    let outro = "{\n   \"speaker_volume\" : 15\n}\n";
    let (c, arquivo) = contexto_com_mira("acrescenta", Some(outro));
    mira(
        &c,
        Some(&serde_json::json!({"zoom_in": [1, 2], "zoom_out": [3, 4]})),
    )
    .unwrap();
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&arquivo).unwrap()).unwrap();
    assert_eq!(doc["speaker_volume"], 15);
    assert_eq!(doc["front_sight_zoom_outy"], 4);
    assert!(
        std::fs::read_to_string(&arquivo)
            .unwrap()
            .ends_with("   \"speaker_volume\" : 15\n}\n")
    );
    for vazio in ["{}", "{\n}\n"] {
        let (c, arquivo) = contexto_com_mira("vazio", Some(vazio));
        mira(
            &c,
            Some(&serde_json::json!({"zoom_in": [1, 2], "zoom_out": [3, 4]})),
        )
        .unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&arquivo).unwrap()).unwrap();
        assert_eq!(doc.as_object().unwrap().len(), 4, "{vazio}");
    }
    let (c, arquivo) = contexto_com_mira("cria", None);
    mira(
        &c,
        Some(&serde_json::json!({"zoom_in": [7, 8], "zoom_out": [9, 10]})),
    )
    .unwrap();
    assert_eq!(
        mira(&c, None).unwrap()["zoom_out"],
        serde_json::json!([9, 10])
    );
    assert!(arquivo.exists());
}

/// Fora da faixa, não inteiro, incompleto, sem `[mira] arquivo` ou com o arquivo estragado: recusa, e o arquivo não muda.
#[test]
fn a_mira_recusa_o_que_nao_e_da_faixa_sem_tocar_no_arquivo() {
    let (c, arquivo) = contexto_com_mira("recusa", Some(AJUSTES_DA_DJI));
    for ruim in [
        serde_json::json!({"zoom_in": [10001, 0], "zoom_out": [0, 0]}),
        serde_json::json!({"zoom_in": [-1, 0], "zoom_out": [0, 0]}),
        serde_json::json!({"zoom_in": [2.5, 0], "zoom_out": [0, 0]}),
        serde_json::json!({"zoom_in": [0, 0]}),
        serde_json::json!({"zoom_in": [0], "zoom_out": [0, 0]}),
        serde_json::json!({"zoom_in": "centro", "zoom_out": [0, 0]}),
    ] {
        assert!(mira(&c, Some(&ruim)).is_err(), "{ruim}");
        assert_eq!(
            std::fs::read_to_string(&arquivo).unwrap(),
            AJUSTES_DA_DJI,
            "{ruim}"
        );
    }
    let e = mira(
        &contexto(),
        Some(&serde_json::json!({"zoom_in": [1, 1], "zoom_out": [1, 1]})),
    )
    .unwrap_err();
    assert!(e.contains("[mira] arquivo"), "{e}");
    let estragado = "{\n   \"front_sight_zoom_inx\" : 5000,\n";
    let (c, arquivo) = contexto_com_mira("estragado", Some(estragado));
    assert!(
        mira(
            &c,
            Some(&serde_json::json!({"zoom_in": [1, 1], "zoom_out": [1, 1]}))
        )
        .is_err()
    );
    assert_eq!(std::fs::read_to_string(&arquivo).unwrap(), estragado);
}

/// Duas conexões gravando ao mesmo tempo: o servidor atende cada uma numa tarefa, num executor de várias threads. O arquivo da
/// DJI termina válido, com o resto byte a byte, e com um dos pares gravados, nunca com pedaços dos dois.
#[test]
fn gravacoes_simultaneas_da_mira_nao_estragam_o_arquivo() {
    let (c, arquivo) = contexto_com_mira("simultaneas", Some(AJUSTES_DA_DJI));
    let c = Arc::new(c);
    for rodada in 0..300 {
        let fios: Vec<_> = (0..4u64)
            .map(|n| {
                let c = Arc::clone(&c);
                std::thread::spawn(move || {
                    let v = 1000 + n * 1000;
                    mira(
                        &c,
                        Some(&serde_json::json!({"zoom_in": [v, v], "zoom_out": [v, v]})),
                    )
                    .is_ok()
                })
            })
            .collect();
        let respostas: Vec<bool> = fios.into_iter().map(|f| f.join().unwrap()).collect();
        let texto = std::fs::read_to_string(&arquivo).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&texto)
            .unwrap_or_else(|e| panic!("rodada {rodada}: arquivo inválido ({e}): {texto:?}"));
        let v = doc["front_sight_zoom_inx"].as_u64().unwrap();
        assert!(
            respostas.iter().all(|r| *r),
            "rodada {rodada}: {respostas:?}"
        );
        assert_eq!(
            texto,
            AJUSTES_DA_DJI.replace(": 5000", &format!(": {v}")),
            "rodada {rodada}"
        );
    }
}

/// Um arquivo que é JSON mas não é objeto está estragado: a leitura recusa, como a gravação, em vez de dizer o padrão.
#[test]
fn ler_a_mira_de_um_arquivo_que_nao_e_objeto_recusa() {
    for ruim in ["[1, 2]", "7", "\"mira\""] {
        let (c, _) = contexto_com_mira("nao-objeto", Some(ruim));
        assert!(mira(&c, None).is_err(), "{ruim}");
    }
}
