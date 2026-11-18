//! O que os testes do agente ligado partilham: um serviço de mentira num socket local, e a espera com prazo.

// Cada arquivo de teste usa uma parte daqui.
#![allow(dead_code)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::broadcast;

/// Um serviço de mentira num socket local: guarda cada linha que chega, responde às requisições, e manda o que o teste
/// pede a quem estiver ligado.
#[derive(Clone)]
pub struct Falso {
    pub recebidas: Arc<Mutex<Vec<Value>>>,
    pub mandar: broadcast::Sender<String>,
    /// Derruba a conexão em curso.
    pub derrubar: broadcast::Sender<()>,
}

impl Falso {
    /// Com `estado`, o serviço passa a mandá-lo a cada 50 ms, como `robot.state`, a quem pedir `robot.subscribe`.
    pub fn subir(socket: &Path, estado: Option<Arc<Mutex<Value>>>) -> Self {
        let ouvinte = UnixListener::bind(socket).unwrap();
        let recebidas = Arc::new(Mutex::new(Vec::new()));
        let (mandar, _) = broadcast::channel(64);
        let (derrubar, _) = broadcast::channel(4);
        let eu = Self {
            recebidas: recebidas.clone(),
            mandar: mandar.clone(),
            derrubar: derrubar.clone(),
        };
        tokio::spawn(async move {
            loop {
                let Ok((conexao, _)) = ouvinte.accept().await else {
                    return;
                };
                let (leitura, mut escrita) = conexao.into_split();
                let mut linhas = BufReader::new(leitura).lines();
                let mut a_mandar = mandar.subscribe();
                let mut cai = derrubar.subscribe();
                let mut ritmo = tokio::time::interval(Duration::from_millis(50));
                let mut assinou = false;
                loop {
                    tokio::select! {
                        l = linhas.next_line() => {
                            let Ok(Some(l)) = l else { break };
                            let v: Value = serde_json::from_str(&l).expect("o agente manda JSON");
                            if let Some(id) = v.get("id") {
                                let r = json!({"jsonrpc": "2.0", "id": id, "result": {"accepted": true}});
                                if escrita.write_all(format!("{r}\n").as_bytes()).await.is_err() { break }
                            }
                            assinou |= v["method"] == "robot.subscribe";
                            recebidas.lock().unwrap().push(v);
                        }
                        Ok(l) = a_mandar.recv() => {
                            if escrita.write_all(format!("{l}\n").as_bytes()).await.is_err() { break }
                        }
                        _ = ritmo.tick(), if assinou && estado.is_some() => {
                            let params = estado.as_ref().unwrap().lock().unwrap().clone();
                            let n = json!({"jsonrpc": "2.0", "method": "robot.state", "params": params});
                            if escrita.write_all(format!("{n}\n").as_bytes()).await.is_err() { break }
                        }
                        _ = cai.recv() => break,
                    }
                }
            }
        });
        eu
    }

    /// As linhas recebidas com o método dado.
    pub fn com(&self, metodo: &str) -> Vec<Value> {
        self.recebidas
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v["method"] == metodo)
            .cloned()
            .collect()
    }

    pub fn notificar(&self, metodo: &str, params: Value) {
        let n = json!({"jsonrpc": "2.0", "method": metodo, "params": params});
        self.mandar.send(n.to_string()).unwrap();
    }
}

/// Espera `cond` valer, com prazo.
pub async fn ate(o_que: &str, mut cond: impl FnMut() -> bool) {
    let fim = tokio::time::Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(
            tokio::time::Instant::now() < fim,
            "não aconteceu em 5 s: {o_que}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
