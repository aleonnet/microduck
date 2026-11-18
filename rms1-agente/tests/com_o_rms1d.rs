//! O agente contra o `rms1d` de verdade: o mesmo servidor e o mesmo laço que vão ao robô, com o corpo falso no lugar da
//! serial. É a prova de que o que o agente manda é o que o `rms1d` entende.

mod comum;

use std::sync::Arc;
use std::time::{Duration, Instant};

use comum::{Falso, ate};
use rms1_agente::ligacoes::{self, Config};
use rms1_agente::tabela::Tabela;
use rms1_control::{CorpoFalso, Estado, Intencoes, Laco, Saude};
use rms1d::respostas::Contexto;
use rms1d_params::{Params, TipoCorpo};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::broadcast;

/// Um cliente do `rms1d`, como a página de quem pilota: chama, e lê a resposta.
struct Piloto {
    linhas: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    saida: tokio::net::unix::OwnedWriteHalf,
    proximo: u64,
}

impl Piloto {
    async fn ligar(socket: &std::path::Path) -> Self {
        let (leitura, saida) = UnixStream::connect(socket).await.unwrap().into_split();
        Self {
            linhas: BufReader::new(leitura).lines(),
            saida,
            proximo: 1,
        }
    }

    async fn chamar(&mut self, metodo: &str, params: Value) -> Value {
        let id = self.proximo;
        self.proximo += 1;
        let pedido = json!({"jsonrpc": "2.0", "id": id, "method": metodo, "params": params});
        self.saida
            .write_all(format!("{pedido}\n").as_bytes())
            .await
            .unwrap();
        loop {
            let l = self
                .linhas
                .next_line()
                .await
                .unwrap()
                .expect("o rms1d fechou");
            let v: Value = serde_json::from_str(&l).unwrap();
            if v["id"] == json!(id) {
                return v;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn o_agente_move_o_corpo_pelo_rms1d_de_verdade() {
    let pasta = tempfile::tempdir().unwrap();
    let (robo, som) = (pasta.path().join("r.sock"), pasta.path().join("m.sock"));
    // O `rms1d`, como o `main` dele o monta, com o corpo falso.
    let mut params = Params::default();
    params.corpo.tipo = TipoCorpo::Falso;
    let (intencoes, saude) = (Arc::new(Intencoes::default()), Arc::new(Saude::default()));
    let (estados, mut ve) = broadcast::channel::<Arc<Estado>>(64);
    let (i, s, e, p) = (
        intencoes.clone(),
        saude.clone(),
        estados.clone(),
        params.clone(),
    );
    std::thread::spawn(move || {
        let mut laco = Laco::new(CorpoFalso::default(), &p, i.clone(), s);
        let periodo = Duration::from_secs_f64(1.0 / f64::from(p.laco.hz));
        while !i.encerrando() {
            if let Some(estado) = laco.ciclo(Instant::now(), true) {
                let _ = e.send(Arc::new(estado));
            }
            std::thread::sleep(periodo);
        }
        laco.encerrar();
    });
    let (_trava, ouvinte) = rms1d::servidor::abrir(&robo).await.unwrap();
    let contexto = Arc::new(Contexto {
        params,
        intencoes: intencoes.clone(),
        saude,
    });
    tokio::spawn(rms1d::servidor::servir(ouvinte, contexto, estados, None));
    // O último pedido de movimento que o laço viu, e se o robô está armado.
    let pedido_ao_laco = Arc::new(std::sync::Mutex::new(([0.0f64; 3], false)));
    // O que o laço aplicou ao corpo no último ciclo: é o que vai a zero quando o pedido vence.
    let aplicado = Arc::new(std::sync::Mutex::new([0.0f64; 3]));
    let (visto, feito) = (pedido_ao_laco.clone(), aplicado.clone());
    tokio::spawn(async move {
        while let Ok(e) = ve.recv().await {
            *visto.lock().unwrap() = (e.pedido, e.armado);
            *feito.lock().unwrap() = e.aplicado;
        }
    });
    let midia = Falso::subir(&som, None);
    let agente = tokio::spawn(ligacoes::rodar(Config {
        robo: robo.clone(),
        midia: som,
        tabela: Tabela::padrao().unwrap(),
    }));
    ate("o agente assinar os pedidos", || {
        !midia.com("s1.media.subscribe").is_empty()
    })
    .await;
    let desfecho = |n: u64| {
        midia
            .com("s1.station.outcome")
            .into_iter()
            .map(|v| v["params"].clone())
            .find(|p| p["request"] == json!(n))
    };
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Desarmado, como o `rms1d` sobe: o agente recusa, e o laço não vê pedido nenhum.
    midia.notificar(
        "s1.station.request",
        json!({"request": 1, "tool": "move", "params": {"direction": "forward", "meters": 0.3}}),
    );
    ate("o desfecho do pedido 1", || desfecho(1).is_some()).await;
    let d = desfecho(1).unwrap();
    assert_eq!(d["outcome"], "recusada", "{d}");
    assert!(d["reason"].as_str().unwrap().contains("armado"), "{d}");
    assert_eq!(pedido_ao_laco.lock().unwrap().0, [0.0; 3]);

    // Quem pilota arma; o agente não.
    let mut piloto = Piloto::ligar(&robo).await;
    let armou = piloto.chamar("robot.enable", json!({"on": true})).await;
    assert_eq!(armou["result"]["accepted"], true, "{armou}");
    ate("o laço armar", || pedido_ao_laco.lock().unwrap().1).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    midia.notificar(
        "s1.station.request",
        json!({"request": 2, "tool": "move", "params": {"direction": "left", "meters": 0.3}}),
    );
    // O laço vê o pedido do agente: 0,3 m/s para a esquerda.
    ate("o laço ver o pedido de andar", || {
        pedido_ao_laco.lock().unwrap().0 == [0.0, 0.3, 0.0]
    })
    .await;
    // O pedido segue valendo além do prazo do `rms1d` (500 ms): o agente o renova, e o laço não deixa de aplicar ao corpo
    // em nenhum instante. (O agente anda 1 s; a amostra para antes.)
    ate("o laço aplicar o pedido ao corpo", || {
        aplicado.lock().unwrap()[1] > 0.0
    })
    .await;
    for _ in 0..70 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(
            aplicado.lock().unwrap()[1] > 0.0,
            "o pedido venceu no rms1d: o laço aplica {:?}",
            aplicado.lock().unwrap()
        );
    }
    // O corpo falso não diz onde está: o agente anda pelo tempo (1 s), para, e diz como terminou.
    ate("o desfecho do pedido 2", || desfecho(2).is_some()).await;
    assert_eq!(desfecho(2).unwrap()["outcome"], "cumprida");
    ate("o laço ver a parada", || {
        pedido_ao_laco.lock().unwrap().0 == [0.0; 3]
    })
    .await;

    // A parada, com o corpo andando de novo: o `rms1d` a aplica na hora.
    midia.notificar(
        "s1.station.request",
        json!({"request": 3, "tool": "move", "params": {"direction": "forward", "meters": 2.0}}),
    );
    ate("o laço ver o pedido de andar", || {
        pedido_ao_laco.lock().unwrap().0 == [0.3, 0.0, 0.0]
    })
    .await;
    midia.notificar(
        "s1.station.request",
        json!({"request": 4, "tool": "stop", "params": {}}),
    );
    ate("os desfechos dos pedidos 3 e 4", || {
        desfecho(3).is_some() && desfecho(4).is_some()
    })
    .await;
    assert_eq!(
        (
            desfecho(3).unwrap()["outcome"].clone(),
            desfecho(4).unwrap()["outcome"].clone()
        ),
        (json!("interrompida"), json!("cumprida"))
    );
    ate("o laço ver a parada", || {
        pedido_ao_laco.lock().unwrap().0 == [0.0; 3]
    })
    .await;
    agente.abort();
    intencoes.pedir_encerrar();
}
