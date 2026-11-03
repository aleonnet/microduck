//! Cada chamada do contrato, decidida. Não há braço genérico: uma chamada nova no contrato para a
//! compilação aqui, e alguém decide o que ela é neste corpo.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use duck_ipc_proto as proto;
use rms1_control::telemetria::{Amostra, BIT_CAPOTOU, BIT_LEVANTADO, BITS_CHASSI, Enlace};
use rms1_control::{Estado, Intencoes, Modo, Saude};
use rms1d_params::{Params, TipoCorpo};

/// O que uma resposta precisa saber.
pub struct Contexto {
    pub params: Params,
    pub intencoes: Arc<Intencoes>,
    pub saude: Arc<Saude>,
}

/// O contrato modela o pato; este corpo responde `roller`, o que está sobre rodas, para `netpadd` usar
/// os limites de rodas.
const MODO_CONTRATO: &str = "roller";
/// `robot.modelApi`: o contrato com o atualizador, igual ao do `s1d`.
const MODEL_API: u32 = 2;

fn nomes_dos_modos() -> Vec<String> {
    Modo::TODOS.iter().map(|m| m.nome().to_owned()).collect()
}

/// `robot.look`: um ponto no corpo vira os dois ângulos da torre. O ângulo limitado é o mais perto que a
/// torre alcança, e `clamped` diz isso.
pub fn olhar(p: &proto::LookParams, s: &rms1d_params::Seguranca) -> proto::LookResult {
    let chao = p.x.hypot(p.y);
    let (yaw, pitch) = if chao == 0.0 && p.z == 0.0 {
        (0.0, 0.0)
    } else {
        (p.y.atan2(p.x), -p.z.atan2(chao))
    };
    let yaw_lim = yaw.clamp(-s.yaw_max_grau.to_radians(), s.yaw_max_grau.to_radians());
    // Contrato: pitch positivo para baixo. Firmware: −20° a 35° com positivo para cima.
    let pitch_lim = pitch.clamp(
        -s.pitch_max_grau.to_radians(),
        -s.pitch_min_grau.to_radians(),
    );
    proto::LookResult {
        head: proto::HeadParams {
            neck_pitch: 0.0,
            head_pitch: pitch_lim,
            head_yaw: yaw_lim,
            head_roll: 0.0,
        },
        clamped: yaw_lim != yaw || pitch_lim != pitch,
    }
}

/// Um pedido contínuo, por notificação ou por requisição. `true` se foi um.
pub fn aplicar_intencao(i: &Intencoes, call: &proto::Call) -> bool {
    match call {
        proto::Call::RobotMove(p) => {
            i.pedir_twist([p.vx, p.vy, p.vyaw]);
            true
        }
        proto::Call::RobotHead(p) => {
            i.pedir_torre(p.head_yaw, p.head_pitch);
            true
        }
        // O `robotd` também aceita a skill como notificação. Nome errado, aqui, não tem a quem responder.
        proto::Call::RobotDo(p) => {
            if let Some(m) = Modo::de_nome(&p.skill) {
                i.pedir_modo(m);
            }
            true
        }
        _ => false,
    }
}

/// `s1.gimbal`, `{"action": "speed", "yaw": graus/s, "pitch": graus/s}`, yaw à direita e pitch para cima, ou `{"action": "stop"}`.
pub fn aplicar_taxa_da_torre(
    i: &Intencoes,
    params: Option<&serde_json::Value>,
) -> Result<(), String> {
    let p = params.ok_or("faltam os parâmetros")?;
    match p["action"].as_str() {
        Some("stop") => i.pedir_taxa_torre(0.0, 0.0),
        Some("speed") => {
            let campo = |nome: &str| {
                p[nome]
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| format!("{nome} tem de ser um número"))
            };
            i.pedir_taxa_torre(campo("yaw")?, campo("pitch")?);
        }
        outra => return Err(format!("ação desconhecida: {outra:?}")),
    }
    Ok(())
}

pub fn saude_do_contrato(c: &Contexto) -> proto::HealthResult {
    let mut v = c.saude.veredito(
        Instant::now(),
        c.params.laco.hz_minimo,
        Duration::from_millis(c.params.laco.idade_maxima_ms),
    );
    let agora = Instant::now();
    let amostra = c.saude.amostra();
    // O controlador empurra telemetria com a DJI parada (`scripts/s1/bench.sh`, medido em 2026-10-30). Sem isto uma serial
    // morta passaria por saudável. O simulador sem página não devolve nada, e isso não é defeito.
    if v.saudavel
        && c.params.corpo.tipo == TipoCorpo::Serial
        && c.saude.segundos() > c.params.laco.serial_muda_ms as f64 / 1000.0
        && c.saude.quadros_recebidos.load(Ordering::Relaxed) == 0
    {
        v.saudavel = false;
        v.degradado = true;
        v.motivo = Some(format!(
            "o corpo está mudo: nenhum quadro recebido em {:.0} s",
            c.saude.segundos()
        ));
    }
    if v.saudavel
        && c.params.corpo.tipo != TipoCorpo::Falso
        && let Some(motivo) = problema_da_telemetria(c, amostra.as_deref(), agora)
    {
        v.saudavel = false;
        v.degradado = true;
        v.motivo = Some(motivo);
    }
    proto::HealthResult {
        healthy: v.saudavel,
        degraded: v.degradado,
        reason: v.motivo,
        control_loop: Some(proto::LoopHealth {
            target_hz: f64::from(c.params.laco.hz),
            achieved_hz: v.hz,
            ticks: v.ciclos,
            missed: v.perdidos,
            last_tick_age_ms: v.idade_ultimo.as_millis() as u64,
        }),
        bus: proto::BusHealth {
            consecutive_errors: v.erros_seguidos,
            ..proto::BusHealth::default()
        },
        // Sem pacote recente, nada: bateria velha não é bateria.
        battery: amostra
            .as_deref()
            .and_then(|a| a.medidas.as_ref())
            .filter(|_| recente(c, agora))
            .map(|m| proto::Battery {
                volts: m.volts,
                percent: m.percentual,
            }),
        ..proto::HealthResult::default()
    }
}

fn recente(c: &Contexto, agora: Instant) -> bool {
    c.saude
        .idade_do_pacote(agora)
        .is_some_and(|i| i < Duration::from_millis(c.params.telemetria.idade_maxima_ms))
}

/// O que dizer sobre a telemetria, se há o que dizer. A palavra "telemetria" é o que `scripts/s1/rms1d-bench.sh` procura: dá
/// para pilotar sem ela, e a ativação avisa e segue.
fn problema_da_telemetria(c: &Contexto, a: Option<&Amostra>, agora: Instant) -> Option<String> {
    let a = a?;
    if let Enlace::Recusado { passo, codigo } = a.enlace {
        return Some(format!(
            "o controlador recusou a telemetria ao {}: código 0x{codigo:02x}, {}",
            passo.nome(),
            rms1_control::telemetria::sentido(codigo)
        ));
    }
    let idade = c.saude.idade_do_pacote(agora);
    if let Some(i) = idade
        && i >= Duration::from_millis(c.params.telemetria.idade_maxima_ms)
    {
        return Some(format!("telemetria parada há {} ms", i.as_millis()));
    }
    // No simulador sem página não há ninguém do outro lado, e isso é o estado normal.
    if let Enlace::SemResposta { passo } = a.enlace
        && c.params.corpo.tipo == TipoCorpo::Serial
    {
        return Some(format!(
            "o controlador não respondeu à telemetria ao {}",
            passo.nome()
        ));
    }
    None
}

/// `s1.state`: tudo o que se sabe do corpo, nos eixos do contrato. O que nunca chegou é `null`, e não zero.
pub fn estado_s1(a: Option<&Amostra>, idade_ms: Option<u64>, recente: bool) -> serde_json::Value {
    use serde_json::json;
    let Some(a) = a else {
        return json!({"link": {"state": "off", "packets": 0, "ignored": 0, "age_ms": idade_ms, "fresh": false}});
    };
    let (codigo, passo) = match a.enlace {
        Enlace::Recusado { passo, codigo } => (Some(codigo), Some(passo.nome())),
        Enlace::SemResposta { passo } => (None, Some(passo.nome())),
        _ => (None, None),
    };
    let chassi: serde_json::Map<String, serde_json::Value> = BITS_CHASSI
        .iter()
        .enumerate()
        .map(|(i, n)| {
            (
                (*n).to_owned(),
                json!({"on": a.estado_chassi & (1 << i) != 0, "count": a.contagem_chassi[i]}),
            )
        })
        .collect();
    let placas: Vec<serde_json::Value> = a
        .placas
        .iter()
        .enumerate()
        .map(|(i, p)| json!({"plate": i + 1, "hits": p.toques, "kind": p.tipo, "mic": p.microfone, "acc": p.aceleracao}))
        .collect();
    json!({
        "link": {
            "state": a.enlace.nome(),
            "code": codigo,
            "code_meaning": codigo.map(rms1_control::telemetria::sentido),
            "step": passo,
            "packets": a.pacotes,
            "ignored": a.ignorados,
            "age_ms": idade_ms,
            "fresh": recente,
        },
        "body": a.medidas.map(|m| json!({
            "velocity_ground": m.velocidade_solo,
            "velocity_body": m.velocidade_corpo,
            "wheels_rpm": m.rodas_rpm,
            "wheels_angle_deg": m.rodas_angulo_grau,
            "wheels_state": m.rodas_estado,
            "position_m": m.posicao,
            "acc_g": m.aceleracao,
            "gyro_rad_s": m.giro,
            "attitude_deg": {"yaw": m.rumo, "pitch": m.inclinacao, "roll": m.rolagem},
            "battery": {"volts": m.volts, "percent": m.percentual, "current_a": m.corrente_a, "temperature_c": m.temperatura_c},
            "robot_clock_ms": m.relogio_ms,
        })),
        // O nosso cálculo, em modo sombra, ao lado do `body` do controlador. O `robot.state` segue com o do controlador.
        "estimate": a.estimativa.map(|e| json!({
            "velocity_body": [e.velocidade_corpo[0], e.velocidade_corpo[1], 0.0],
            "position_m": [e.posicao[0], e.posicao[1], 0.0],
            "yaw_deg": e.rumo,
            "picked_up": e.levantado,
            "disagreement_m_s": e.discordancia,
            "gravity": e.gravidade,
            "tilt_divergence_deg": e.divergencia,
            "tilt_divergent": e.divergente,
        })),
        "gimbal": a.torre.map(|t| json!({
            "yaw_deg": t.rumo,
            "pitch_deg": t.inclinacao,
            "yaw_ground_deg": t.rumo_solo,
            "pitch_ground_deg": t.inclinacao_solo,
        })),
        "chassis": chassi,
        "plates": placas,
        "ir": {"top_left": a.infravermelho[0], "top_right": a.infravermelho[1]},
    })
}

/// O estado do laço no formato do contrato.
pub fn estado_do_contrato(e: &Estado) -> proto::RobotState {
    let [mut yaw, mut pitch] = e.torre.unwrap_or([0.0; 2]);
    let corpo = e.corpo.as_deref().filter(|_| e.corpo_recente);
    // Com pacote recente, `head` é onde a torre está; sem, é o último alvo, e nunca um dado velho.
    if let Some(t) = corpo.and_then(|a| a.torre.as_ref()) {
        yaw = t.rumo.to_radians();
        pitch = -t.inclinacao.to_radians();
    }
    let medidas = corpo.and_then(|a| a.medidas.as_ref());
    let chassi = corpo.map_or(0, |a| a.estado_chassi);
    proto::RobotState {
        t: e.t,
        movement: proto::MoveState {
            requested: e.pedido,
            applied: e.aplicado,
            limited_by: e.limitado_por.iter().map(|s| (*s).to_owned()).collect(),
        },
        head: [0.0, pitch, yaw, 0.0],
        policy: if e.armado { e.modo.nome() } else { "held" }.to_owned(),
        safety: proto::SafetyState {
            fallen: chassi & BIT_CAPOTOU != 0,
            limp: false,
            gravity: medidas.map_or([0.0, 0.0, -1.0], |m| m.gravidade()),
            gain: None,
            picked_up: chassi & BIT_LEVANTADO != 0,
        },
        control_loop: proto::LoopState {
            hz: e.hz,
            missed: e.perdidos,
        },
        joints: Vec::new(),
        targets: Vec::new(),
        velocities: Vec::new(),
        currents_ma: Vec::new(),
        odom: medidas.map_or_else(proto::OdomState::default, |m| proto::OdomState {
            position: m.posicao,
            yaw: m.rumo.to_radians(),
        }),
        theremin: None,
        chorale: None,
        t_ns: proto::clock::monotonic_ns(),
        imu: medidas.map(|m| proto::ImuState {
            gyro: m.giro,
            quat: m.quaternion(),
        }),
        frames: None,
        skeleton: Vec::new(),
    }
}

fn nao_servida(id: proto::Id, call: &proto::Call) -> proto::Response {
    proto::Response::err(
        Some(id),
        proto::Error::new(
            proto::code::METHOD_NOT_FOUND,
            format!("{} não é servida por rms1d", call.method()),
        ),
    )
}

/// Responder uma requisição. Síncrona e sem esperar o laço: estas respostas têm de sair com tudo quebrado.
pub fn responder(c: &Contexto, id: proto::Id, call: &proto::Call) -> proto::Response {
    use proto::Call::*;
    let aceito = |id| proto::Response::ok(Some(id), &proto::IntentResult::accepted());
    let recusado =
        |id, por: &str| proto::Response::ok(Some(id), &proto::IntentResult::refused(por));
    match call {
        Hello(_) => proto::Response::ok(
            Some(id),
            &proto::HelloResult {
                api_version: proto::API_VERSION,
                daemon_version: proto::semver::Version::parse(env!("CARGO_PKG_VERSION")).ok(),
                revision: proto::build_info!().revision.map(str::to_owned),
            },
        ),
        SystemInfo => proto::Response::ok(
            Some(id),
            &proto::SystemInfoResult {
                name: c.params.robo.nome.clone(),
                serial: None,
                uptime_seconds: c.saude.segundos() as u64,
                simulated: c.params.corpo.tipo != TipoCorpo::Serial,
            },
        ),
        // A página mostra a escolha de dados anônimos só se o robô responde. Este corpo não envia nada.
        TelemetryStatus => proto::Response::ok(Some(id), &proto::TelemetryResult::default()),

        RobotMove(_) | RobotHead(_) => {
            aplicar_intencao(&c.intencoes, call);
            aceito(id)
        }
        RobotLook(p) => {
            let r = olhar(p, &c.params.seguranca);
            c.intencoes.pedir_torre(r.head.head_yaw, r.head.head_pitch);
            proto::Response::ok(Some(id), &r)
        }
        RobotStop => {
            c.intencoes.parar();
            aceito(id)
        }
        RobotEnable(p) => {
            let sim = if p.toggle {
                !c.intencoes.armado_pedido()
            } else {
                p.on
            };
            c.intencoes.pedir_armar(sim);
            proto::Response::ok(
                Some(id),
                &proto::IntentResult {
                    accepted: true,
                    reason: Some(if sim { "armed" } else { "disarmed" }.to_owned()),
                },
            )
        }
        RobotInit => {
            c.intencoes.pedir_armar(true);
            aceito(id)
        }
        RobotRelax => {
            c.intencoes.pedir_armar(false);
            aceito(id)
        }
        RobotHealth => proto::Response::ok(Some(id), &saude_do_contrato(c)),
        RobotMode => proto::Response::ok(
            Some(id),
            &proto::ModeResult {
                mode: MODO_CONTRATO.to_owned(),
            },
        ),
        RobotPolicies => proto::Response::ok(
            Some(id),
            &proto::PoliciesResult {
                mode: MODO_CONTRATO.to_owned(),
                enabled: c.intencoes.armado_pedido(),
                skills: nomes_dos_modos(),
                ..proto::PoliciesResult::default()
            },
        ),
        RobotSkills => proto::Response::ok(
            Some(id),
            &proto::SkillsResult {
                skills: Vec::new(),
                built_in: nomes_dos_modos(),
            },
        ),
        RobotDo(p) => match Modo::de_nome(&p.skill) {
            Some(m) => {
                c.intencoes.pedir_modo(m);
                aceito(id)
            }
            None => recusado(id, &format!("não há skill chamada {}", p.skill)),
        },
        RobotSubscribe(_) => proto::Response::ok(
            Some(id),
            &proto::SubscribeResult {
                accepted: true,
                unavailable: Some(
                    "este corpo não roda rede de política: o chassi anda por cinemática".to_owned(),
                ),
                skills: nomes_dos_modos(),
                ..proto::SubscribeResult::default()
            },
        ),
        RobotSafeToRestart => {
            let a = c.intencoes.ler(Instant::now());
            let prazo = Duration::from_millis(c.params.seguranca.prazo_ms);
            let andando = c.intencoes.armado_pedido()
                && a.twist != [0.0; 3]
                && a.idade_twist.is_some_and(|i| i < prazo);
            proto::Response::ok(
                Some(id),
                &proto::SafeToRestartResult {
                    safe: !andando,
                    reason: andando.then(|| "o chassi está sendo pilotado".to_owned()),
                },
            )
        }
        RobotModelApi => proto::Response::ok(
            Some(id),
            &proto::ModelApiResult {
                model_api: MODEL_API,
            },
        ),
        RobotRemoteSessionActive => {
            proto::Response::ok(Some(id), &serde_json::json!({"active": false}))
        }
        PadBindings => proto::Response::ok(
            Some(id),
            &proto::PadBindingsResult {
                bindings: Vec::new(),
            },
        ),

        // Do corpo, em etapas seguintes da lista do dono.
        RobotSound(_) => recusado(id, "som entra na etapa 4"),
        RobotFlashlight(_) => recusado(id, "luzes entram na etapa 4"),
        // Do pato, sem equivalente neste corpo.
        RobotPose(_) => recusado(id, "este corpo não tem pernas"),
        RobotMouth(_) => recusado(id, "este corpo não tem boca"),
        RobotRest | RobotShutdown => {
            recusado(id, "este corpo não senta nem se desliga; use robot.relax")
        }
        RobotSetMode(_) => recusado(id, "este corpo está sempre sobre rodas"),
        RobotTheremin(_) => recusado(id, "este corpo não tem sensor de profundidade"),
        RobotChorale(_) | ChoraleSubscribe => recusado(id, "este corpo não canta"),
        // Estas duas não respondem `accepted`, e sim um documento: uma recusa viraria um documento vazio no cliente.
        RobotModel | RobotVetbook => nao_servida(id, call),
        RobotVetbookNote(_) | RobotVetbookTally(_) => recusado(id, "este corpo não mantém caderno"),
        RobotLoadPolicy(_) | RobotReloadPolicies | RobotSetSkill(_) | RobotRemoveSkill(_)
        | PadBind(_) => recusado(id, "este corpo não roda rede de política"),
        RobotRebootMotors(_) => recusado(
            id,
            "os motores estão atrás do controlador de movimento; desligue na chave",
        ),

        // De outros serviços: o console os chama em sockets próprios, e aqui o nome é a resposta.
        Check(_)
        | Apply(_)
        | Rollback(_)
        | ResetToGolden(_)
        | Select(_)
        | Pin(_)
        | Status
        | ListInstalled(_)
        | Log(_)
        | Show(_)
        | Subscribe
        | ChoraleBeaconSet(_)
        | ChoraleHeard(_)
        | PolicyCheck
        | PolicyInstall(_)
        | PolicyFetch(_)
        | PolicySearch(_)
        | DetectorCheck
        | DetectorInstall(_)
        | AccountLogin(_)
        | AccountStatus
        | AccountLogout
        | VetbookShare(_)
        | OsStatus
        | OsCheck
        | OsApply(_)
        | OsSwitch(_)
        | TelemetrySet(_)
        | NetStatus
        | NetScan
        | NetConnect(_)
        | NetForget(_)
        | SystemServices
        | SystemLogs(_)
        | SystemSetName(_)
        | SystemReboot
        | SystemPairingPin
        | SystemSetPairingPin(_)
        | SystemAuthenticate(_)
        | PadStatus
        | PadPair(_)
        | PadForget(_)
        | PadInput
        | TofStream
        | HeadImuStream => nao_servida(id, call),
    }
}
