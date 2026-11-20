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
use serde_json::{Value, json};

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
/// dizendo quem assumiu, e nada vai ao corpo: nem parada, nem rodas a zero, nem renovação.
#[test]
fn o_piloto_assume_as_rodas_e_a_habilidade_termina_sem_mexer_no_corpo() {
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
    let [Acao::Terminou(d)] = acoes.as_slice() else {
        panic!("a habilidade segue em curso, ou saiu algum pedido ao corpo: {acoes:?}")
    };
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
    assert_eq!(a.donos(EU, Some("piloto"), None, t0 + ms(1000)).len(), 1);
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
        let [Acao::Terminou(d)] = acoes.as_slice() else {
            panic!("{ferramenta}: {acoes:?}")
        };
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
        1,
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
    assert!(
        corpo
            .com("robot.move")
            .iter()
            .all(|v| v["params"]["vx"] == 0.3),
        "saiu um pedido de rodas a zero: o canal já era do piloto"
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
