//! O agente e a tabela de fontes do `rms1d`: ele diz quem é, lê quem tem as rodas e a torre, e quando outra fonte assume
//! o canal da habilidade em curso ele a encerra dizendo quem assumiu, sem mandar nada ao corpo.

mod comum;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use comum::{Falso, ate};
use rms1_agente::agente::{Acao, Agente, Como, Corpo, NOME_NO_CORPO, Pedido};
use rms1_agente::ligacoes::{self, Config};
use rms1_agente::tabela::{PERDEU_APOS_PADRAO_MS, Tabela};
use rms1_control::{CorpoFalso, Estado, Intencoes, Laco, Saude};
use rms1d::respostas::Contexto;
use rms1d_params::{Params, TipoCorpo};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::broadcast;

/// A linha da tabela que o `rms1d` diz que a conexão do agente é.
const EU: &str = NOME_NO_CORPO;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn pedido(ferramenta: &str, parametros: Value) -> Pedido {
    Pedido {
        numero: 7,
        ferramenta: ferramenta.to_owned(),
        parametros: parametros.as_object().cloned().unwrap_or_default(),
    }
}

/// Um agente com uma habilidade em curso, começada em `t0`.
fn em_curso(ferramenta: &str, parametros: Value, t0: Instant) -> Agente {
    let mut a = Agente::novo(Tabela::padrao().unwrap());
    a.corpo(
        Corpo {
            armado: true,
            posicao: [0.0; 2],
        },
        t0,
    );
    a.pedido(pedido(ferramenta, parametros), t0);
    assert_eq!(a.em_curso(), Some(ferramenta));
    a
}

fn andando(t0: Instant) -> Agente {
    em_curso("move", json!({"direction": "forward", "meters": 2.0}), t0)
}

/// O prazo é o da tabela que vem com o agente: 300 ms, três renovações do pedido. A tabela que não o diz vale o mesmo, e
/// zero é recusado pelo nome.
#[test]
fn o_prazo_de_perder_o_canal_vem_da_tabela() {
    let t = Tabela::padrao().unwrap();
    assert_eq!(t.geral.perdeu_apos_ms, 300);
    assert_eq!(
        PERDEU_APOS_PADRAO_MS, t.geral.perdeu_apos_ms,
        "o padrão do código e o de habilidades.toml se afastaram"
    );
    assert_eq!(t.geral.perdeu_apos_ms, 3 * t.geral.renovar_ms);
    let sem = "[geral]\nrenovar_ms = 100\ncorpo_vale_ms = 1000\n[[habilidade]]\nfaz = \"parar\"\nferramenta = \"stop\"\n";
    assert_eq!(Tabela::ler(sem).unwrap().geral.perdeu_apos_ms, 300);
    let outro = sem.replace(
        "corpo_vale_ms = 1000",
        "corpo_vale_ms = 1000\nperdeu_apos_ms = 150",
    );
    assert_eq!(Tabela::ler(&outro).unwrap().geral.perdeu_apos_ms, 150);
    let zero = sem.replace(
        "corpo_vale_ms = 1000",
        "corpo_vale_ms = 1000\nperdeu_apos_ms = 0",
    );
    let e = Tabela::ler(&zero).unwrap_err();
    assert!(e.contains("perdeu_apos_ms"), "{e}");
}

/// Aceite 12: o agente está andando e o piloto assume as rodas. Passado o prazo, a habilidade termina interrompida,
/// dizendo quem assumiu. Ao corpo vai só o zero na linha do agente, que apaga o último pedido que ele renovou: nem
/// parada, que pararia o piloto, nem renovação.
#[test]
fn o_piloto_assume_as_rodas_e_a_habilidade_termina_soltando_so_a_linha_dele() {
    let t0 = Instant::now();
    let mut a = andando(t0);
    // Enquanto as rodas são do agente, ou de ninguém, nada muda.
    assert!(
        a.donos(EU, Some(NOME_NO_CORPO), None, t0 + ms(50))
            .is_empty()
    );
    assert!(a.donos(EU, None, None, t0 + ms(100)).is_empty());
    // O piloto pegou. Dentro do prazo, a habilidade segue em curso.
    assert!(a.donos(EU, Some("piloto"), None, t0 + ms(200)).is_empty());
    assert!(a.donos(EU, Some("piloto"), None, t0 + ms(499)).is_empty());
    assert_eq!(a.em_curso(), Some("move"), "terminou antes do prazo");
    let acoes = a.donos(EU, Some("piloto"), None, t0 + ms(500));
    let [Acao::Corpo(solta), Acao::Terminou(d)] = acoes.as_slice() else {
        panic!("a habilidade segue em curso, ou o agente não soltou as rodas: {acoes:?}")
    };
    assert_eq!(
        (&solta["method"], &solta["params"], solta.get("id")),
        (
            &json!("robot.move"),
            &json!({"vx": 0.0, "vy": 0.0, "vyaw": 0.0}),
            None
        ),
        "o zero na linha dele, por notificação, e não uma parada"
    );
    assert_eq!(
        (d.numero, d.ferramenta.as_str(), d.como),
        (7, "move", Como::Interrompida)
    );
    assert_eq!(
        d.motivo, "o piloto assumiu as rodas",
        "termina sem o motivo"
    );
    assert_eq!(a.em_curso(), None);
    // Não retoma sozinha, nem quando o piloto solta: nenhuma renovação sai.
    assert!(a.tique(t0 + ms(600)).is_empty());
    assert!(a.donos(EU, None, None, t0 + ms(1200)).is_empty());
    assert!(a.tique(t0 + ms(1300)).is_empty());
}

/// Um instante com outra fonte não conta: o prazo recomeça quando as rodas voltam ao agente, ou a ninguém.
#[test]
fn um_instante_com_outra_fonte_nao_interrompe() {
    let t0 = Instant::now();
    let mut a = andando(t0);
    for (dono, em) in [
        (Some("piloto"), 100),
        (Some("piloto"), 350),
        (Some(NOME_NO_CORPO), 380),
        (Some("piloto"), 400),
        (Some("piloto"), 650),
        (None, 680),
        (Some("piloto"), 700),
        (Some("piloto"), 950),
    ] {
        assert!(
            a.donos(EU, dono, None, t0 + ms(em)).is_empty(),
            "aos {em} ms"
        );
    }
    assert_eq!(a.em_curso(), Some("move"));
    assert_eq!(a.donos(EU, Some("piloto"), None, t0 + ms(1000)).len(), 2);
}

/// O canal é o da habilidade: quem gira a torre não liga para quem tem as rodas, e quem anda não liga para a torre.
#[test]
fn cada_habilidade_olha_o_canal_dela() {
    let t0 = Instant::now();
    let mut a = andando(t0);
    for n in 0..10 {
        assert!(
            a.donos(EU, Some(NOME_NO_CORPO), Some("piloto"), t0 + ms(100 * n))
                .is_empty()
        );
    }
    assert_eq!(
        a.em_curso(),
        Some("move"),
        "a torre do piloto parou quem andava"
    );
    for (ferramenta, alvo) in [("search", "dog"), ("look_at", "me")] {
        let mut a = em_curso(ferramenta, json!({"target": alvo}), t0);
        for n in 0..10 {
            assert!(
                a.donos(EU, Some("piloto"), Some(NOME_NO_CORPO), t0 + ms(100 * n))
                    .is_empty()
            );
        }
        assert_eq!(
            a.em_curso(),
            Some(ferramenta),
            "as rodas do piloto pararam a torre"
        );
        assert!(a.donos(EU, None, Some("cerebro"), t0 + ms(1000)).is_empty());
        let acoes = a.donos(EU, None, Some("cerebro"), t0 + ms(1300));
        let [Acao::Corpo(solta), Acao::Terminou(d)] = acoes.as_slice() else {
            panic!("{ferramenta}: {acoes:?}")
        };
        assert_eq!(
            (&solta["method"], &solta["params"]),
            (&json!("s1.gimbal"), &json!({"action": "stop"})),
            "{ferramenta}: quem girava a torre solta a torre"
        );
        assert_eq!(d.como, Como::Interrompida);
        assert_eq!(d.motivo, "o cerebro assumiu a torre");
    }
    // Numa tabela sem a linha do agente, o `rms1d` diz que ele é a fonte sem nome: o dono com esse nome é ele mesmo, e
    // não outra fonte. Sem isto o agente se daria por tomado por si próprio.
    let mut a = andando(t0);
    for n in 0..10 {
        assert!(
            a.donos("sem_nome", Some("sem_nome"), None, t0 + ms(100 * n))
                .is_empty(),
            "o agente se deu por tomado por ele mesmo"
        );
    }
    assert_eq!(a.em_curso(), Some("move"));
    assert!(
        a.donos("sem_nome", Some("agente"), None, t0 + ms(1000))
            .is_empty()
    );
    assert_eq!(
        a.donos("sem_nome", Some("agente"), None, t0 + ms(1300))
            .len(),
        2,
        "e outra fonte, com o nome que for, é outra"
    );
    // Sem habilidade em curso, não há o que perder.
    let mut a = Agente::novo(Tabela::padrao().unwrap());
    assert!(a.donos(EU, Some("piloto"), Some("piloto"), t0).is_empty());
    assert!(
        a.donos(EU, Some("piloto"), Some("piloto"), t0 + ms(1000))
            .is_empty()
    );
}

fn estado_do_corpo() -> Value {
    json!({"policy": "modo_livre", "odom": {"position": [0.0, 0.0, 0.0], "yaw": 0.0}})
}

/// Com os sockets: o agente diz quem é antes de tudo, assina `s1.state`, e é pelo bloco `control` dele que sabe que o
/// piloto assumiu. A estação recebe o desfecho com o motivo, e o corpo não recebe parada nem pedido novo.
#[tokio::test(flavor = "multi_thread")]
async fn o_agente_le_o_dono_das_rodas_no_s1_state() {
    let pasta = tempfile::tempdir().unwrap();
    let (robo, som): (PathBuf, PathBuf) =
        (pasta.path().join("r.sock"), pasta.path().join("m.sock"));
    let corpo = Falso::subir(&robo, Some(Arc::new(Mutex::new(estado_do_corpo()))));
    let midia = Falso::subir(&som, None);
    let agente = tokio::spawn(ligacoes::rodar(Config {
        robo,
        midia: som,
        tabela: Tabela::padrao().unwrap(),
    }));
    ate("o agente ligar aos dois", || {
        !corpo.com("s1.subscribe").is_empty() && !midia.com("s1.media.subscribe").is_empty()
    })
    .await;
    // As três linhas de abertura, nesta ordem: o `hello` é o que põe as seguintes na linha do agente.
    let abertura: Vec<Value> = corpo.recebidas.lock().unwrap()[..3].to_vec();
    let metodos: Vec<&str> = abertura
        .iter()
        .map(|v| v["method"].as_str().unwrap())
        .collect();
    assert_eq!(metodos, ["hello", "robot.subscribe", "s1.subscribe"]);
    assert_eq!(
        abertura[0]["params"],
        json!({"api_version": duck_ipc_proto::API_VERSION, "client": "agente"}),
        "só os dois campos: o hello do contrato recusa campo desconhecido"
    );
    tokio::time::sleep(Duration::from_millis(150)).await;
    let controle = |rodas: Value| json!({"link": {"state": "off"}, "control": {"client": "agente", "wheels": {"source": rodas}, "gimbal": {"source": null}, "autonomous": false}});
    midia.notificar(
        "s1.station.request",
        json!({"request": 61, "tool": "move", "params": {"direction": "forward", "meters": 2.0}}),
    );
    ate("o corpo receber o pedido de andar", || {
        !corpo.com("robot.move").is_empty()
    })
    .await;
    let desfecho = || {
        midia
            .com("s1.station.outcome")
            .into_iter()
            .map(|v| v["params"].clone())
            .find(|p| p["request"] == json!(61))
    };
    // As rodas são do agente: segue andando.
    for _ in 0..8 {
        corpo.notificar("s1.state", controle(json!("agente")));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(desfecho().is_none(), "terminou com as rodas na mão dele");
    // O piloto assumiu.
    for _ in 0..10 {
        corpo.notificar("s1.state", controle(json!("piloto")));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    ate("o desfecho do pedido 61", || desfecho().is_some()).await;
    let d = desfecho().unwrap();
    assert_eq!(
        d["outcome"], "interrompida",
        "a habilidade segue em curso: {d}"
    );
    assert_eq!(d["reason"], "o piloto assumiu as rodas");
    let andou = corpo.com("robot.move").len();
    let pedidos: Vec<Value> = corpo
        .com("robot.move")
        .iter()
        .map(|v| v["params"]["vx"].clone())
        .collect();
    assert!(
        pedidos[..andou - 1].iter().all(|vx| *vx == 0.3) && pedidos[andou - 1] == 0.0,
        "o agente renova o pedido e, ao desistir, zera a linha dele uma vez: {pedidos:?}"
    );
    assert!(
        corpo.com("robot.stop").is_empty(),
        "o agente parou quem pilota"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        corpo.com("robot.move").len(),
        andou,
        "seguiu renovando o pedido"
    );
    agente.abort();
}

/// O agente de verdade contra o `rms1d` de verdade (o servidor e o laço que vão ao robô, com o corpo falso e a tabela de
/// produção): o agente anda, e quem pilota manda parar. O corpo para, o agente larga o pedido, e **o corpo segue parado
/// depois que a vez do piloto vence**. Sem o agente soltar a linha dele ao desistir, o último pedido que ele renovou ficava
/// guardado no `rms1d` e passava a valer meio segundo depois da parada: o robô dava um tranco para a frente (o leitor frio
/// do diff, 2026-11-20).
#[tokio::test(flavor = "multi_thread")]
async fn depois_da_parada_do_piloto_o_pedido_do_agente_nao_volta_sozinho() {
    let pasta = tempfile::tempdir().unwrap();
    let (robo, som) = (pasta.path().join("r.sock"), pasta.path().join("m.sock"));
    let mut params = Params::default();
    params.corpo.tipo = TipoCorpo::Falso;
    let prazo = Duration::from_millis(params.seguranca.prazo_ms);
    let (intencoes, saude) = (
        Arc::new(Intencoes::com_fontes(&params.fontes, prazo)),
        Arc::new(Saude::default()),
    );
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
    // O que o laço aplicou a cada ciclo, com a hora: é aqui que o tranco apareceria.
    let aplicou = Arc::new(Mutex::new(Vec::<(Instant, [f64; 3])>::new()));
    let visto = aplicou.clone();
    tokio::spawn(async move {
        while let Ok(e) = ve.recv().await {
            visto.lock().unwrap().push((Instant::now(), e.aplicado));
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
    // Quem pilota: apresenta-se como a página, arma, e mais tarde manda parar.
    let (leitura, mut piloto) = UnixStream::connect(&robo).await.unwrap().into_split();
    let mut respostas = BufReader::new(leitura).lines();
    for linha in [
        json!({"jsonrpc": "2.0", "id": 1, "method": "hello", "params": {"api_version": duck_ipc_proto::API_VERSION, "client": "piloto"}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "robot.enable", "params": {"on": true}}),
    ] {
        piloto
            .write_all(format!("{linha}\n").as_bytes())
            .await
            .unwrap();
        respostas.next_line().await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    midia.notificar(
        "s1.station.request",
        json!({"request": 71, "tool": "move", "params": {"direction": "forward", "meters": 2.0}}),
    );
    ate("o corpo andar pelo pedido do agente", || {
        aplicou
            .lock()
            .unwrap()
            .last()
            .is_some_and(|(_, v)| v[0] > 0.0)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    // A parada de quem pilota.
    let parou = Instant::now();
    let parada = json!({"jsonrpc": "2.0", "id": 3, "method": "robot.stop"});
    piloto
        .write_all(format!("{parada}\n").as_bytes())
        .await
        .unwrap();
    let desfecho = || {
        midia
            .com("s1.station.outcome")
            .into_iter()
            .map(|v| v["params"].clone())
            .find(|p| p["request"] == json!(71))
    };
    ate("o desfecho do pedido 71", || desfecho().is_some()).await;
    let d = desfecho().unwrap();
    assert_eq!(d["outcome"], "interrompida", "{d}");
    assert_eq!(d["reason"], "o piloto assumiu as rodas");
    // Bem além do prazo do piloto (500 ms) mais o do último pedido que o agente renovou: o corpo não pode andar de novo.
    tokio::time::sleep((parou + prazo * 3).saturating_duration_since(Instant::now())).await;
    let andou: Vec<(u128, f64)> = aplicou
        .lock()
        .unwrap()
        .iter()
        .filter(|(t, v)| *t > parou + Duration::from_millis(100) && v[0] != 0.0)
        .map(|(t, v)| (t.duration_since(parou).as_millis(), v[0]))
        .collect();
    assert!(
        andou.is_empty(),
        "o corpo andou sozinho depois da parada do piloto, com o pedido já interrompido: {} ciclos, de {:?} a {:?} (ms depois da parada, m/s)",
        andou.len(),
        andou.first(),
        andou.last()
    );
    intencoes.pedir_encerrar();
    agente.abort();
}
