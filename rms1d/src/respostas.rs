//! Cada chamada do contrato, decidida. Não há braço genérico: uma chamada nova no contrato para a
//! compilação aqui, e alguém decide o que ela é neste corpo.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use duck_ipc_proto as proto;
use rms1_control::telemetria::{Amostra, BIT_CAPOTOU, BIT_LEVANTADO, BITS_CHASSI, Enlace};
use rms1_control::{Estado, FonteId, Intencoes, Modo, Saude};
use rms1d_params::{Canal, Params, TipoCorpo};

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

/// Por que a fonte não pode pedir neste canal; `None` se pode. A parada não passa por aqui: vale para qualquer fonte.
pub fn barrado(i: &Intencoes, fonte: FonteId, canal: Canal) -> Option<String> {
    let nome = i.nome(fonte);
    if !i.pode(fonte, canal) {
        return Some(format!(
            "a fonte {nome} não tem o canal {} na tabela de rms1d.toml",
            canal.nome()
        ));
    }
    i.espera_a_autonomia(fonte)
        .then(|| format!("a fonte {nome} só vale com a autonomia ligada (robot.autonomous)"))
}

/// A fonte de prioridade maior que tem as rodas ou a torre agora, em palavras, para a recusa.
fn superado(i: &Intencoes, fonte: FonteId, canal: Canal) -> Option<String> {
    let o_que = match canal {
        Canal::Rodas => "as rodas",
        _ => "a torre",
    };
    i.quem_supera(fonte, canal, Instant::now())
        .map(|maior| format!("{} tem {o_que} agora", i.nome(maior)))
}

/// O que foi feito de um pedido de intenção.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Feito {
    /// A chamada não é um pedido contínuo.
    NaoEra,
    Aceito,
    /// Não vale, com o motivo: a tabela não dá o canal à fonte, ela espera a autonomia, ou perde para uma fonte maior.
    Recusado(String),
}

/// Quem manda agora: o bloco `control` de `s1.state` e de `s1.telemetry`. Cada nome é o da fonte com pedido vivo naquele
/// canal; ausente, ninguém o tem.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Controle {
    /// A linha da tabela que a conexão que lê é (`client`): o nome dito no `hello`, ou a fonte sem nome se a tabela não
    /// o tem. É com ele que um cliente sabe se o dono de um canal é ele mesmo, sem supor que o nome dele está na tabela.
    pub cliente: Option<Arc<str>>,
    pub rodas: Option<Arc<str>>,
    pub torre: Option<Arc<str>>,
    pub autonomia: bool,
}

impl Controle {
    /// O do ciclo que o laço publicou, para a conexão desta fonte.
    pub fn do_estado(e: &Estado, cliente: Arc<str>) -> Self {
        Self {
            cliente: Some(cliente),
            rodas: e.dono_rodas.clone(),
            torre: e.dono_torre.clone(),
            autonomia: e.autonomia,
        }
    }

    /// O de agora, sem esperar o laço, para a conexão desta fonte.
    pub fn de_agora(i: &Intencoes, fonte: FonteId, agora: Instant) -> Self {
        let l = i.ler(agora);
        Self {
            cliente: Some(i.nome(fonte)),
            rodas: l.dono_rodas.map(|f| i.nome(f)),
            torre: l.dono_torre.map(|f| i.nome(f)),
            autonomia: i.autonomia(),
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "client": self.cliente.as_deref(),
            "wheels": {"source": self.rodas.as_deref()},
            "gimbal": {"source": self.torre.as_deref()},
            "autonomous": self.autonomia,
        })
    }
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
            alpha: None,
        },
        clamped: yaw_lim != yaw || pitch_lim != pitch,
    }
}

/// Um pedido contínuo da fonte, por notificação ou por requisição.
///
/// `com_resposta` é se o cliente espera a resposta. Aí o pedido de rodas que perde para uma fonte de prioridade maior é
/// recusado e não fica guardado: dizer "recusado" e aplicá-lo meio segundo depois, quando a outra soltasse, seria mentir.
/// Por notificação ele fica guardado, e passa a valer quando a outra soltar: é assim que quem renova o pedido retoma o
/// corpo. O alvo de ângulo da torre é um só, e quem perde não o troca, com resposta ou sem.
pub fn aplicar_intencao(
    i: &Intencoes,
    fonte: FonteId,
    call: &proto::Call,
    com_resposta: bool,
) -> Feito {
    let canal = match call {
        proto::Call::RobotMove(_) => Canal::Rodas,
        proto::Call::RobotHead(_) => Canal::Torre,
        proto::Call::RobotDo(_) => Canal::Modo,
        _ => return Feito::NaoEra,
    };
    if let Some(por) = barrado(i, fonte, canal) {
        return Feito::Recusado(por);
    }
    match call {
        proto::Call::RobotMove(p) => {
            if com_resposta && let Some(por) = superado(i, fonte, Canal::Rodas) {
                return Feito::Recusado(por);
            }
            i.pedir_twist(fonte, [p.vx, p.vy, p.vyaw]);
        }
        proto::Call::RobotHead(p) => {
            if !i.pedir_torre(fonte, p.head_yaw, p.head_pitch) {
                return Feito::Recusado(torre_de_outra(i, fonte));
            }
        }
        // O `robotd` também aceita a skill como notificação. Nome errado, aqui, não tem a quem responder.
        proto::Call::RobotDo(p) => {
            if let Some(m) = Modo::de_nome(&p.skill) {
                i.pedir_modo(m);
            }
        }
        _ => {}
    }
    Feito::Aceito
}

/// O motivo de o alvo da torre não ter sido trocado.
fn torre_de_outra(i: &Intencoes, fonte: FonteId) -> String {
    superado(i, fonte, Canal::Torre).unwrap_or_else(|| "outra fonte tem a torre agora".to_owned())
}

/// `s1.gimbal`, `{"action": "speed", "yaw": graus/s, "pitch": graus/s}`, yaw à direita e pitch para cima, ou `{"action": "stop"}`.
/// O erro é o do pedido malformado; `com_resposta` vale como em [`aplicar_intencao`].
pub fn aplicar_taxa_da_torre(
    i: &Intencoes,
    fonte: FonteId,
    params: Option<&serde_json::Value>,
    com_resposta: bool,
) -> Result<Feito, String> {
    let p = params.ok_or("faltam os parâmetros")?;
    let [yaw, pitch] = match p["action"].as_str() {
        Some("stop") => [0.0; 2],
        Some("speed") => {
            let campo = |nome: &str| {
                p[nome]
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| format!("{nome} tem de ser um número"))
            };
            [campo("yaw")?, campo("pitch")?]
        }
        outra => return Err(format!("ação desconhecida: {outra:?}")),
    };
    if let Some(por) = barrado(i, fonte, Canal::Torre) {
        return Ok(Feito::Recusado(por));
    }
    if com_resposta && let Some(por) = superado(i, fonte, Canal::Torre) {
        return Ok(Feito::Recusado(por));
    }
    i.pedir_taxa_torre(fonte, yaw, pitch);
    Ok(Feito::Aceito)
}

/// `s1.armor` e `s1.sight` com valores gravam no robô, e isso é o canal `ajustes`. Sem valores só leem, e qualquer fonte
/// lê. O motivo, se a fonte não pode gravar.
pub fn ajuste_barrado(
    i: &Intencoes,
    fonte: FonteId,
    metodo: &str,
    params: Option<&serde_json::Value>,
) -> Option<String> {
    let grava = match metodo {
        "s1.armor" => sensibilidade_pedida(params).is_some(),
        "s1.sight" => mira_pedida(params).is_some(),
        _ => false,
    };
    grava.then(|| barrado(i, fonte, Canal::Ajustes)).flatten()
}

fn sensibilidade_pedida(params: Option<&serde_json::Value>) -> Option<&serde_json::Value> {
    params.and_then(|p| p.get("sensitivity"))
}

fn mira_pedida(params: Option<&serde_json::Value>) -> Option<&serde_json::Value> {
    params.filter(|p| p.get("zoom_in").is_some() || p.get("zoom_out").is_some())
}

/// `s1.armor`: a sensibilidade das placas, de 0 a 10, a faixa da DJI (`RobotArmor.set_hit_sensitivity`). Com `sensitivity`, o
/// laço a manda às seis placas; sem, só responde a atual: a pedida, ou a do arquivo.
pub fn sensibilidade_das_placas(
    c: &Contexto,
    params: Option<&serde_json::Value>,
) -> Result<serde_json::Value, String> {
    if let Some(v) = sensibilidade_pedida(params) {
        let n = v
            .as_u64()
            .filter(|n| *n <= 10)
            .ok_or("sensitivity tem de ser um inteiro de 0 a 10, a faixa da DJI")?;
        c.intencoes.pedir_sensibilidade(n as u8);
    }
    let atual = c
        .intencoes
        .sensibilidade()
        .unwrap_or(c.params.armadura.sensibilidade);
    Ok(serde_json::json!({ "sensitivity": atual }))
}

/// As quatro posições da mira, com zoom x e y, sem zoom x e y, em décimos de milésimo da tela: as chaves de
/// `/data/rm_user_setting.json`. A ordem é a das chaves no `dji_hdvt_uav` e dos quatro campos seguidos do leitor de `0x3f/0x57`
/// (`0x1d670`), não conferida campo a campo.
const CHAVES_DA_MIRA: [&str; 4] = [
    "front_sight_zoom_inx",
    "front_sight_zoom_iny",
    "front_sight_zoom_outx",
    "front_sight_zoom_outy",
];
/// O padrão de fábrica: o `dji_hdvt_uav` grava 5000 nos quatro campos antes de ler o arquivo (`0x1d2f8`), o centro da tela.
const MIRA_PADRAO: u64 = 5000;
/// A tela inteira, nas unidades da DJI: o firmware divide por 10000 (`get_sight_bead_position`, `rm_ctrl.py`).
const MIRA_MAXIMO: u64 = 10000;

/// `s1.sight`: a mira da DJI. Sem valores, responde as quatro posições do arquivo de `[mira] arquivo` (o padrão onde faltar o
/// arquivo ou a chave); com `zoom_in` e `zoom_out`, cada um `[x, y]` de 0 a 10000, grava e responde o que ficou.
pub fn mira(c: &Contexto, params: Option<&serde_json::Value>) -> Result<serde_json::Value, String> {
    let arquivo = c.params.mira.arquivo.as_deref().map(std::path::Path::new);
    if let Some(p) = mira_pedida(params) {
        let par = |nome: &str| -> Result<[u64; 2], String> {
            let v = p
                .get(nome)
                .and_then(|v| v.as_array())
                .filter(|a| a.len() == 2)
                .ok_or(format!("{nome} tem de ser [x, y]"))?;
            let n = |i: usize| {
                v[i].as_u64().filter(|n| *n <= MIRA_MAXIMO).ok_or(format!(
                    "{nome} tem de ter inteiros de 0 a 10000, a tela inteira da DJI"
                ))
            };
            Ok([n(0)?, n(1)?])
        };
        let (zi, zo) = (par("zoom_in")?, par("zoom_out")?);
        let arquivo =
            arquivo.ok_or("sem [mira] arquivo em rms1d.toml: não há onde gravar a mira")?;
        gravar_mira(arquivo, [zi[0], zi[1], zo[0], zo[1]])?;
    }
    let v = ler_mira(arquivo)?;
    Ok(serde_json::json!({ "zoom_in": [v[0], v[1]], "zoom_out": [v[2], v[3]] }))
}

fn ler_mira(arquivo: Option<&std::path::Path>) -> Result<[u64; 4], String> {
    let mut v = [MIRA_PADRAO; 4];
    let Some(arquivo) = arquivo else { return Ok(v) };
    let texto = match std::fs::read_to_string(arquivo) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(v),
        Err(e) => return Err(format!("{}: {e}", arquivo.display())),
    };
    // Um JSON que não é objeto está estragado: recusa, como a gravação, em vez de dizer o padrão.
    let doc: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&texto).map_err(|e| format!("{}: {e}", arquivo.display()))?;
    for (i, chave) in CHAVES_DA_MIRA.iter().enumerate() {
        if let Some(n) = doc.get(*chave) {
            v[i] = n.as_u64().filter(|n| *n <= MIRA_MAXIMO).ok_or(format!(
                "{}: {chave} fora de 0 a 10000: {n}",
                arquivo.display()
            ))?;
        }
    }
    Ok(v)
}

/// Uma gravação de cada vez: o servidor atende cada conexão numa tarefa, num executor de várias threads, e duas gravações com o
/// mesmo temporário estragavam o arquivo da DJI (o leitor frio de 2026-11-05 mediu 14 em 3000 rodadas).
static GRAVANDO_A_MIRA: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Grava as quatro posições mexendo só nos números: o resto do arquivo, que é da DJI, fica byte a byte (o `serde_json` daqui não
/// guarda a ordem das chaves). O resultado é relido e conferido antes de trocar o arquivo, por um temporário e `rename`, com o
/// dono e as permissões do original.
fn gravar_mira(arquivo: &std::path::Path, v: [u64; 4]) -> Result<(), String> {
    let _vez = GRAVANDO_A_MIRA
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let erro = |e: &dyn std::fmt::Display| format!("{}: {e}", arquivo.display());
    let texto = match std::fs::read_to_string(arquivo) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "{\n}\n".to_owned(),
        Err(e) => return Err(erro(&e)),
    };
    let antes: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&texto).map_err(|e| erro(&e))?;
    let mut novo = texto.clone();
    let mut faltam = Vec::new();
    for (chave, valor) in CHAVES_DA_MIRA.iter().zip(v) {
        match numero_da_chave(&novo, chave) {
            Some((ini, fim)) => novo.replace_range(ini..fim, &valor.to_string()),
            None => faltam.push(format!("   \"{chave}\" : {valor}")),
        }
    }
    if !faltam.is_empty() {
        let abre = novo
            .find('{')
            .ok_or_else(|| erro(&"o arquivo não é um objeto"))?
            + 1;
        let resto = if antes.is_empty() { "\n" } else { ",\n" };
        novo.insert_str(abre, &format!("\n{}{resto}", faltam.join(",\n")));
        if antes.is_empty() {
            // O objeto vazio fecha logo depois: tira o que sobrou entre as chaves novas e o fecho.
            let fecho = novo
                .rfind('}')
                .ok_or_else(|| erro(&"o arquivo não é um objeto"))?;
            let ultimo = novo[..fecho].trim_end().len();
            novo.replace_range(ultimo..fecho, "\n");
        }
    }
    let depois: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&novo)
        .map_err(|e| erro(&format!("a gravação daria um arquivo inválido: {e}")))?;
    let iguais = antes
        .iter()
        .filter(|(k, _)| !CHAVES_DA_MIRA.contains(&k.as_str()))
        .all(|(k, x)| depois.get(k) == Some(x));
    let certas = CHAVES_DA_MIRA
        .iter()
        .zip(v)
        .all(|(k, n)| depois.get(*k).and_then(|x| x.as_u64()) == Some(n));
    if !iguais || !certas || depois.len() != antes.len() + faltam.len() {
        return Err(erro(
            &"a gravação mudaria outra parte do arquivo da DJI; nada foi gravado",
        ));
    }
    let temporario = arquivo.with_extension("json.rms1d");
    let original = std::fs::metadata(arquivo).ok();
    let gravar = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&temporario)?;
        std::io::Write::write_all(&mut f, novo.as_bytes())?;
        if let Some(m) = &original {
            use std::os::unix::fs::MetadataExt;
            f.set_permissions(m.permissions())?;
            std::os::unix::fs::fchown(&f, Some(m.uid()), Some(m.gid()))?;
        }
        f.sync_all()?;
        std::fs::rename(&temporario, arquivo)
    };
    gravar().map_err(|e| {
        let _ = std::fs::remove_file(&temporario);
        erro(&e)
    })
}

/// Onde está o número de `"chave" : n` no texto, do primeiro dígito ao último.
fn numero_da_chave(texto: &str, chave: &str) -> Option<(usize, usize)> {
    let aspas = format!("\"{chave}\"");
    let depois = texto.find(&aspas)? + aspas.len();
    let resto = &texto[depois..];
    let dois_pontos = resto.find(|c: char| !c.is_whitespace())?;
    if !resto[dois_pontos..].starts_with(':') {
        return None;
    }
    let apos = &resto[dois_pontos + 1..];
    let ini = apos.find(|c: char| !c.is_whitespace())?;
    let fim = apos[ini..]
        .find(|c: char| !c.is_ascii_digit())
        .map_or(apos.len(), |f| ini + f);
    if fim == ini {
        return None;
    }
    let base = depois + dois_pontos + 1;
    Some((base + ini, base + fim))
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

/// `s1.state`: tudo o que se sabe do corpo, nos eixos do contrato, e quem manda nele. O que nunca chegou é `null`, e não
/// zero. O bloco `control` sai sempre, também sem telemetria: quem dirige não depende de o corpo falar.
pub fn estado_s1(
    a: Option<&Amostra>,
    idade_ms: Option<u64>,
    recente: bool,
    controle: &Controle,
) -> serde_json::Value {
    use serde_json::json;
    let Some(a) = a else {
        return json!({
            "link": {"state": "off", "packets": 0, "ignored": 0, "age_ms": idade_ms, "fresh": false},
            "control": controle.json(),
        });
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
        "control": controle.json(),
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
            // Como no pato: o nome de quem fez o pedido de `requested`, e nada se ele não disse o nome.
            source: e
                .fonte_do_pedido
                .as_deref()
                .filter(|n| *n != rms1d_params::FONTE_SEM_NOME)
                .map(str::to_owned),
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
        // Do pato, sem equivalente neste corpo: a guarda de impacto das pernas, o laser, o futebol e a escuta.
        impact_guard: Vec::new(),
        odom: medidas.map_or_else(proto::OdomState::default, |m| proto::OdomState {
            position: m.posicao,
            yaw: m.rumo.to_radians(),
        }),
        theremin: None,
        laser: None,
        soccer: None,
        autonomous: e.autonomia,
        chorale: None,
        t_ns: proto::clock::monotonic_ns(),
        imu: medidas.map(|m| proto::ImuState {
            gyro: m.giro,
            quat: m.quaternion(),
        }),
        frames: None,
        skeleton: Vec::new(),
        hearing: None,
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

/// Responder uma requisição da fonte. Síncrona e sem esperar o laço: estas respostas têm de sair com tudo quebrado.
pub fn responder(
    c: &Contexto,
    fonte: FonteId,
    id: proto::Id,
    call: &proto::Call,
) -> proto::Response {
    use proto::Call::*;
    let aceito = |id| proto::Response::ok(Some(id), &proto::IntentResult::accepted());
    let recusado =
        |id, por: &str| proto::Response::ok(Some(id), &proto::IntentResult::refused(por));
    let i = &c.intencoes;
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

        RobotMove(_) | RobotHead(_) => match aplicar_intencao(i, fonte, call, true) {
            Feito::Recusado(por) => recusado(id, &por),
            Feito::Aceito | Feito::NaoEra => aceito(id),
        },
        // Responde um documento, e não `accepted`: quem não pode recebe erro, ou acreditaria que a torre foi.
        RobotLook(p) => {
            let r = olhar(p, &c.params.seguranca);
            if let Some(por) = barrado(i, fonte, Canal::Torre) {
                proto::Response::err(
                    Some(id),
                    proto::Error::new(proto::code::PERMISSION_DENIED, por),
                )
            } else if i.pedir_torre(fonte, r.head.head_yaw, r.head.head_pitch) {
                proto::Response::ok(Some(id), &r)
            } else {
                proto::Response::err(
                    Some(id),
                    proto::Error::new(proto::code::BUSY, torre_de_outra(i, fonte)),
                )
            }
        }
        // A parada vale para qualquer fonte: não há canal a conferir.
        RobotStop => {
            i.parar(fonte);
            aceito(id)
        }
        RobotEnable(_) | RobotInit | RobotRelax if barrado(i, fonte, Canal::Energia).is_some() => {
            recusado(id, &barrado(i, fonte, Canal::Energia).unwrap_or_default())
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
                autonomous: i.autonomia(),
                ..proto::ModeResult::default()
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
        RobotDo(p) => match (Modo::de_nome(&p.skill), barrado(i, fonte, Canal::Modo)) {
            (None, _) => recusado(id, &format!("não há skill chamada {}", p.skill)),
            (Some(_), Some(por)) => recusado(id, &por),
            (Some(m), None) => {
                c.intencoes.pedir_modo(m);
                aceito(id)
            }
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
            // O pedido de quem tem as rodas, julgado como a segurança o julga: o prazo da fonte dele, ou o de `[seguranca]`.
            let a = c.intencoes.ler(Instant::now());
            let prazo = a
                .prazo_twist
                .unwrap_or(Duration::from_millis(c.params.seguranca.prazo_ms));
            let andando = c.intencoes.armado_pedido()
                && a.twist != [0.0; 3]
                && a.idade_twist.is_some_and(|i| i <= prazo);
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
        PadBindings => proto::Response::ok(
            Some(id),
            &proto::PadBindingsResult {
                bindings: Vec::new(),
            },
        ),

        // Do corpo, em etapas seguintes da lista do dono.
        // Com `[midia]`, o servidor repassa ao `rms1-mediad` antes de chegar aqui (`servidor.rs`).
        RobotSound(_) => recusado(id, "sem [midia] em rms1d.toml: quem toca é o rms1-mediad"),
        RobotFlashlight(_) => recusado(id, "luzes entram na etapa 4"),
        // Do pato, sem equivalente neste corpo.
        RobotPose(_) => recusado(id, "este corpo não tem pernas"),
        RobotMouth(_) => recusado(id, "este corpo não tem boca"),
        RobotRest | RobotShutdown => {
            recusado(id, "este corpo não senta nem se desliga; use robot.relax")
        }
        RobotSetMode(_) => recusado(id, "este corpo está sempre sobre rodas"),
        RobotTheremin(_) => recusado(id, "este corpo não tem sensor de profundidade"),
        RobotLaserTrack(_) | RobotSoccer(_) => recusado(id, "este corpo não tem o jogo do pato"),
        // O robô por conta própria: um nível que este serviço guarda e publica (`robot.mode` e `robot.state`), e que
        // libera as fontes `so_autonomo` da tabela. Quem o muda precisa do canal `autonomia`.
        RobotAutonomous(p) => match barrado(i, fonte, Canal::Autonomia) {
            Some(por) => recusado(id, &por),
            None => {
                if i.autonomia() != p.active {
                    tracing::warn!(ligada = p.active, por = %i.nome(fonte), "a autonomia mudou");
                }
                i.definir_autonomia(p.active);
                aceito(id)
            }
        },
        RobotChorale(_) | ChoraleSubscribe => recusado(id, "este corpo não canta"),
        // Estas duas não respondem `accepted`, e sim um documento: uma recusa viraria um documento vazio no cliente.
        RobotModel | RobotVetbook => nao_servida(id, call),
        // O volume também responde um documento (`VolumeResult`). No S1 quem tem o alto-falante é o `rms1-mediad`.
        RobotVolume | RobotSetVolume(_) => proto::Response::err(
            Some(id),
            proto::Error::new(
                proto::code::METHOD_NOT_FOUND,
                format!(
                    "{} não é servida por rms1d: o volume do S1 é do rms1-mediad",
                    call.method()
                ),
            ),
        ),
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
        | HeadImuStream
        | NfcWrite(_)
        | NfcRead(_)
        | PhotoList
        | PhotoGet(_)
        | PhotoDelete(_)
        | PhotoTake
        | MapStatus
        | MapStream
        | MapGrid(_)
        | MapWipe => nao_servida(id, call),
    }
}
