//! O som: o `rms1d` não toca, repassa `robot.sound` ao `rms1-mediad`. Os critérios 10 e 11 do plano, com o processo de verdade
//! e um `rms1-mediad` de mentira, que só anota o que recebe.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const RMS1D: &str = env!("CARGO_BIN_EXE_rms1d");

/// Curto de propósito: caminho de socket tem limite de tamanho.
fn pasta(nome: &str) -> PathBuf {
    let d = PathBuf::from(format!("/tmp/rms1m-{}-{nome}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Processo(Child, PathBuf);

impl Drop for Processo {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn subir(d: &Path, toml: &str) -> Processo {
    let cfg = d.join("rms1d.toml");
    std::fs::write(&cfg, toml).unwrap();
    let socket = d.join("r.sock");
    let filho = Command::new(RMS1D)
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "--socket",
            socket.to_str().unwrap(),
        ])
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let inicio = Instant::now();
    while UnixStream::connect(&socket).is_err() {
        assert!(
            inicio.elapsed() < Duration::from_secs(5),
            "o rms1d não subiu"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    Processo(filho, socket)
}

/// Um pedido com resposta: a resposta e quanto ela demorou.
fn chamar(socket: &Path, metodo: &str, params: Value) -> (Value, Duration) {
    let mut c = UnixStream::connect(socket).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let inicio = Instant::now();
    writeln!(
        c,
        "{}",
        json!({"jsonrpc": "2.0", "id": 1, "method": metodo, "params": params})
    )
    .unwrap();
    let mut linha = String::new();
    BufReader::new(c).read_line(&mut linha).unwrap();
    (serde_json::from_str(&linha).unwrap(), inicio.elapsed())
}

/// O `rms1-mediad` de mentira: aceita conexões e anota cada linha que chega.
fn mediad_de_mentira(socket: &Path) -> Arc<Mutex<Vec<Value>>> {
    let ouvinte = UnixListener::bind(socket).unwrap();
    let recebido = Arc::new(Mutex::new(Vec::new()));
    let r = recebido.clone();
    std::thread::spawn(move || {
        for conexao in ouvinte.incoming().flatten() {
            let r = r.clone();
            std::thread::spawn(move || {
                for linha in BufReader::new(conexao).lines().map_while(Result::ok) {
                    r.lock()
                        .unwrap()
                        .push(serde_json::from_str(&linha).unwrap());
                }
            });
        }
    });
    recebido
}

#[test]
fn repassa_o_som() {
    let d = pasta("rep");
    let midia = d.join("m.sock");
    let recebido = mediad_de_mentira(&midia);
    let p = subir(
        &d,
        &format!(
            "[corpo]\ntipo = \"falso\"\n[midia]\nsocket = \"{}\"\n",
            midia.display()
        ),
    );
    // O `rms1d` liga ao `rms1-mediad` sozinho: até ligar, recusa dizendo isso.
    let inicio = Instant::now();
    let r = loop {
        let (r, _) = chamar(&p.1, "robot.sound", json!({"tag": "chirp"}));
        if r["result"]["accepted"] == true {
            break r;
        }
        assert!(
            inicio.elapsed() < Duration::from_secs(5),
            "não ligou ao rms1-mediad: {r}"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(r["result"]["accepted"], true, "{r}");
    // O `wheee` segurado vai com o `hold`, e a notificação, sem resposta, também é repassada.
    let (r, _) = chamar(&p.1, "robot.sound", json!({"tag": "wheee", "hold": true}));
    assert_eq!(r["result"]["accepted"], true, "{r}");
    let mut c = UnixStream::connect(&p.1).unwrap();
    writeln!(c, "{}", json!({"jsonrpc": "2.0", "method": "robot.sound", "params": {"tag": "wheee", "hold": false}})).unwrap();
    let inicio = Instant::now();
    while recebido.lock().unwrap().len() < 3 {
        assert!(
            inicio.elapsed() < Duration::from_secs(3),
            "{:?}",
            recebido.lock().unwrap()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let r = recebido.lock().unwrap().clone();
    assert!(
        r.iter()
            .all(|m| m["method"] == "robot.sound" && m.get("id").is_none()),
        "repassado como notificação: {r:?}"
    );
    assert_eq!(
        r.iter().map(|m| m["params"].clone()).collect::<Vec<_>>(),
        [
            json!({"tag": "chirp"}),
            json!({"tag": "wheee", "hold": true}),
            json!({"tag": "wheee", "hold": false})
        ]
    );
    // Um som que o contrato não tem é recusado aqui, e não chega ao `rms1-mediad`.
    let (r, _) = chamar(&p.1, "robot.sound", json!({"tag": "latido"}));
    assert!(r["error"].is_object(), "{r}");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(recebido.lock().unwrap().len(), 3);
}

#[test]
fn sem_mediad_recusa_com_motivo() {
    // Com `[midia]` e sem o `rms1-mediad` no ar.
    let d = pasta("sem");
    let p = subir(
        &d,
        &format!(
            "[corpo]\ntipo = \"falso\"\n[midia]\nsocket = \"{}\"\n",
            d.join("m.sock").display()
        ),
    );
    let (r, demorou) = chamar(&p.1, "robot.sound", json!({"tag": "chirp"}));
    assert_eq!(r["result"]["accepted"], false, "{r}");
    assert!(
        r["result"]["reason"]
            .as_str()
            .unwrap()
            .contains("rms1-mediad não está no ar"),
        "{r}"
    );
    assert!(
        demorou < Duration::from_millis(100),
        "a recusa demorou {demorou:?}"
    );
    drop(p);

    // Sem `[midia]`.
    let d = pasta("nada");
    let p = subir(&d, "[corpo]\ntipo = \"falso\"\n");
    let (r, demorou) = chamar(&p.1, "robot.sound", json!({"tag": "chirp"}));
    assert_eq!(r["result"]["accepted"], false, "{r}");
    assert!(
        r["result"]["reason"]
            .as_str()
            .unwrap()
            .contains("sem [midia]"),
        "{r}"
    );
    assert!(
        demorou < Duration::from_millis(100),
        "a recusa demorou {demorou:?}"
    );
}
