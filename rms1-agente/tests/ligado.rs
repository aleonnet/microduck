//! O agente ligado: um `rms1d` e um `rms1-mediad` de mentira, nos sockets locais, e o agente de verdade entre os dois.

mod comum;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use comum::{Falso, ate};
use rms1_agente::ligacoes::{self, Config};
use rms1_agente::tabela::Tabela;
use serde_json::{Value, json};

fn estado_do_corpo(armado: bool, x: f64) -> Value {
    json!({"policy": if armado { "modo_livre" } else { "held" }, "odom": {"position": [x, 0.0, 0.0], "yaw": 0.0}})
}

struct Bancada {
    _pasta: tempfile::TempDir,
    corpo: Falso,
    midia: Falso,
    estado: Arc<Mutex<Value>>,
    agente: tokio::task::JoinHandle<()>,
}

impl Bancada {
    async fn subir() -> Self {
        let pasta = tempfile::tempdir().unwrap();
        let (robo, som): (PathBuf, PathBuf) =
            (pasta.path().join("r.sock"), pasta.path().join("m.sock"));
        let estado = Arc::new(Mutex::new(estado_do_corpo(true, 0.0)));
        let corpo = Falso::subir(&robo, Some(estado.clone()));
        let midia = Falso::subir(&som, None);
        let cfg = Config {
            robo,
            midia: som,
            tabela: Tabela::padrao().unwrap(),
        };
        let agente = tokio::spawn(ligacoes::rodar(cfg));
        let b = Self {
            _pasta: pasta,
            corpo,
            midia,
            estado,
            agente,
        };
        // O agente diz quem é, e assina o estado do corpo e quem tem cada canal; e os pedidos e as detecções da mídia.
        ate("o agente assinar o estado do corpo", || {
            !b.corpo.com("robot.subscribe").is_empty() && !b.corpo.com("s1.subscribe").is_empty()
        })
        .await;
        let primeira = b.corpo.recebidas.lock().unwrap()[0].clone();
        assert_eq!(
            (&primeira["method"], &primeira["params"]["client"]),
            (&json!("hello"), &json!("agente")),
            "o hello vem antes de tudo: {primeira}"
        );
        ate("o agente assinar os pedidos e as detecções", || {
            b.midia
                .com("s1.media.subscribe")
                .last()
                .is_some_and(|v| v["params"] == json!({"requests": true, "detections": true}))
        })
        .await;
        // O primeiro estado do corpo já chegou ao agente.
        tokio::time::sleep(Duration::from_millis(150)).await;
        b
    }

    /// O desfecho que o agente mandou do pedido de número `n`, se já mandou.
    fn desfecho(&self, n: u64) -> Option<Value> {
        self.midia
            .com("s1.station.outcome")
            .into_iter()
            .map(|v| v["params"].clone())
            .find(|p| p["request"] == json!(n))
    }
}

impl Drop for Bancada {
    fn drop(&mut self) {
        self.agente.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn o_pedido_da_estacao_move_o_corpo_e_o_desfecho_volta_a_estacao() {
    let b = Bancada::subir().await;
    // A estação manda cumprir: o corpo recebe o pedido de andar, renovado, na velocidade da tabela.
    b.midia.notificar(
        "s1.station.request",
        json!({"request": 41, "tool": "move", "params": {"direction": "forward", "meters": 0.3}}),
    );
    ate("o corpo receber o pedido de andar, renovado", || {
        b.corpo.com("robot.move").len() >= 3
    })
    .await;
    assert_eq!(
        b.corpo.com("robot.move")[0]["params"],
        json!({"vx": 0.3, "vy": 0.0, "vyaw": 0.0})
    );
    assert!(b.desfecho(41).is_none(), "terminou antes de chegar");
    // O corpo chegou: o agente solta as rodas, e a estação fica sabendo como terminou.
    *b.estado.lock().unwrap() = estado_do_corpo(true, 0.3);
    ate("o desfecho do pedido 41", || b.desfecho(41).is_some()).await;
    let d = b.desfecho(41).unwrap();
    assert_eq!(
        (d["tool"].as_str(), d["outcome"].as_str()),
        (Some("move"), Some("cumprida"))
    );
    assert!(
        d["reason"].as_str().is_some_and(|m| m.contains("0.30")),
        "{d}"
    );
    assert_eq!(
        b.corpo.com("robot.stop").len(),
        0,
        "o fim de uma habilidade não é uma parada"
    );
    let andou = b.corpo.com("robot.move").len();
    assert_eq!(
        b.corpo.com("robot.move")[andou - 1]["params"],
        json!({"vx": 0.0, "vy": 0.0, "vyaw": 0.0}),
        "as rodas soltas"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(b.corpo.com("robot.move").len(), andou, "seguiu andando");

    // Com o robô desarmado, o pedido é recusado, e o corpo não recebe nada.
    *b.estado.lock().unwrap() = estado_do_corpo(false, 0.3);
    tokio::time::sleep(Duration::from_millis(150)).await;
    b.midia.notificar(
        "s1.station.request",
        json!({"request": 42, "tool": "move", "params": {"direction": "back", "meters": 0.3}}),
    );
    ate("o desfecho do pedido 42", || b.desfecho(42).is_some()).await;
    assert_eq!(b.desfecho(42).unwrap()["outcome"], "recusada");
    assert_eq!(b.corpo.com("robot.move").len(), andou);
    // A parada passa assim mesmo.
    b.midia.notificar(
        "s1.station.request",
        json!({"request": 43, "tool": "stop", "params": {}}),
    );
    ate("o desfecho do pedido 43", || b.desfecho(43).is_some()).await;
    assert_eq!(b.desfecho(43).unwrap()["outcome"], "cumprida");
    assert_eq!(
        b.corpo.com("robot.stop").len(),
        1,
        "a da ferramenta de parar"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn as_deteccoes_guiam_a_torre_e_o_corpo_que_cai_interrompe() {
    let b = Bancada::subir().await;
    b.midia.notificar(
        "s1.station.request",
        json!({"request": 51, "tool": "look_at", "params": {"target": "me"}}),
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Uma pessoa à direita do meio do quadro: a torre vai para a direita.
    let deteccao = json!({"width": 640, "height": 360, "took_ms": 30.0, "boxes": [
        {"x0": 500.0, "y0": 150.0, "x1": 600.0, "y1": 210.0, "score": 0.9, "class": 0, "label": "person"}]});
    b.midia.notificar("media.detections", deteccao);
    ate("a torre receber a velocidade", || {
        !b.corpo.com("s1.gimbal").is_empty()
    })
    .await;
    let p = b.corpo.com("s1.gimbal")[0]["params"].clone();
    assert_eq!(p["action"], "speed");
    assert!(
        p["yaw"].as_f64().unwrap() > 0.0 && p["pitch"].as_f64().unwrap() == 0.0,
        "{p}"
    );
    // O `rms1d` cai no meio: a habilidade termina interrompida, e o agente liga de novo quando ele volta.
    let assinaturas = b.corpo.com("robot.subscribe").len();
    b.corpo.derrubar.send(()).unwrap();
    ate("o desfecho do pedido 51", || b.desfecho(51).is_some()).await;
    assert_eq!(b.desfecho(51).unwrap()["outcome"], "interrompida");
    ate("o agente ligar de novo ao corpo", || {
        b.corpo.com("robot.subscribe").len() > assinaturas
    })
    .await;
    // A estação manda largar um pedido em curso: as rodas são soltas, e o desfecho diz que foi interrompido.
    tokio::time::sleep(Duration::from_millis(150)).await;
    b.midia.notificar(
        "s1.station.request",
        json!({"request": 53, "tool": "move", "params": {"direction": "forward", "meters": 2.0}}),
    );
    ate("o corpo receber o pedido de andar", || {
        !b.corpo.com("robot.move").is_empty()
    })
    .await;
    let paradas = b.corpo.com("robot.stop").len();
    b.midia
        .notificar("s1.station.request", json!({"request": 53, "cancel": true}));
    ate("o desfecho do pedido 53", || b.desfecho(53).is_some()).await;
    assert_eq!(b.desfecho(53).unwrap()["outcome"], "interrompida");
    assert_eq!(b.corpo.com("robot.stop").len(), paradas);
    assert_eq!(
        b.corpo.com("robot.move").last().unwrap()["params"],
        json!({"vx": 0.0, "vy": 0.0, "vyaw": 0.0})
    );
    // O detector de patos não diz rótulo: a caixa dele é um pato.
    tokio::time::sleep(Duration::from_millis(150)).await;
    b.midia.notificar(
        "s1.station.request",
        json!({"request": 52, "tool": "search", "params": {"target": "the duck"}}),
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    b.midia.notificar(
        "media.detections",
        json!({"width": 640, "height": 360, "took_ms": 30.0, "boxes": [{"x0": 10.0, "y0": 10.0, "x1": 60.0, "y1": 60.0, "score": 0.8}]}),
    );
    ate("o desfecho do pedido 52", || b.desfecho(52).is_some()).await;
    assert_eq!(b.desfecho(52).unwrap()["outcome"], "cumprida");
}
