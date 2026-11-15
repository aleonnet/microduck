//! O quadro sob pedido: os critérios 1 a 4 do plano.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use duck_ipc_proto as proto;
use rms1_mediad::camera::{self, NV12, TESTE_ALTURA, TESTE_LARGURA};
use rms1_mediad::quadros::Quadros;
use rms1_mediad::servidor;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

const PEDIDO: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"media.frame\"}\n";

/// A fonte de teste numa thread, como no serviço; devolve quem a para.
fn fonte(quadros: &Quadros) -> Arc<AtomicBool> {
    let parar = Arc::new(AtomicBool::new(false));
    let (q, p) = (quadros.clone(), parar.clone());
    std::thread::spawn(move || camera::rodar_teste(q, Duration::from_millis(5), p));
    parar
}

/// Espera alguém pedir um quadro, como a fonte faz, e leva o pedido. Com prazo: um encontro quebrado reprova, e não trava.
fn esperar_pedido(quadros: &Quadros) {
    let limite = std::time::Instant::now() + Duration::from_secs(5);
    while !quadros.levar_pedido() {
        assert!(
            std::time::Instant::now() < limite,
            "ninguém pediu um quadro"
        );
        std::thread::yield_now();
    }
}

/// Um pedido de `media.frame` pelo socket: o cabeçalho e os bytes que vieram depois dele.
async fn pedir(quadros: Quadros) -> (proto::Response, Vec<u8>) {
    let (mut cliente, servidor_) = UnixStream::pair().unwrap();
    let tarefa = tokio::spawn(servidor::atender(servidor_, quadros));
    cliente.write_all(PEDIDO).await.unwrap();
    let mut leitor = BufReader::new(cliente);
    let mut linha = String::new();
    leitor.read_line(&mut linha).await.unwrap();
    let resposta: proto::Response =
        serde_json::from_str(linha.trim()).expect("uma resposta JSON-RPC");
    let mut bytes = Vec::new();
    leitor.read_to_end(&mut bytes).await.unwrap();
    tarefa.await.unwrap().unwrap();
    (resposta, bytes)
}

#[tokio::test(flavor = "multi_thread")]
async fn quadro_de_teste_tem_cabecalho_e_bytes() {
    let quadros = Quadros::default();
    let parar = fonte(&quadros);
    let (resposta, bytes) = pedir(quadros).await;
    parar.store(true, std::sync::atomic::Ordering::Relaxed);
    let r = resposta.result.expect("um quadro, e não um erro");
    assert_eq!(r["format"], NV12);
    assert_eq!(
        (r["width"].as_u64(), r["height"].as_u64()),
        (Some(256), Some(144))
    );
    assert_eq!(r["bytes"], 55_296);
    assert_eq!(
        bytes.len(),
        55_296,
        "depois do cabeçalho, exatamente os bytes do quadro"
    );
    // Os bytes são os de um quadro de teste: a croma neutra e a rampa de luma de alguma fase.
    let luma = (TESTE_LARGURA * TESTE_ALTURA) as usize;
    assert!(bytes[luma..].iter().all(|&b| b == 128), "croma neutra");
    let fase = (0..TESTE_LARGURA as u64).find(|&n| camera::quadro_de_teste(n).dados == bytes);
    assert!(
        fase.is_some(),
        "os bytes não são os de nenhum quadro de teste"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sem_quadro_no_prazo_e_erro() {
    // Um quadro entregue antes do pedido, e depois a fonte para: o pedido não pode receber o quadro velho.
    let quadros = Quadros::default();
    assert!(!quadros.levar_pedido());
    let q = quadros.clone();
    let antes = std::thread::spawn(move || q.proximo());
    esperar_pedido(&quadros);
    quadros.entregar(camera::quadro_de_teste(0));
    assert!(
        antes.join().unwrap().is_some(),
        "o primeiro pedido recebe a captura feita depois dele"
    );
    let (resposta, bytes) = pedir(quadros).await;
    assert!(
        resposta.result.is_none(),
        "chegou um resultado sem captura nova"
    );
    assert_eq!(
        resposta.error.expect("um erro").code,
        proto::code::INTERNAL_ERROR
    );
    assert!(bytes.is_empty(), "nenhum byte de quadro depois de um erro");
}

#[test]
fn dois_pedidos_uma_copia() {
    let quadros = Quadros::default();
    let espera: Vec<_> = (0..2)
        .map(|_| {
            let q = quadros.clone();
            std::thread::spawn(move || q.proximo_em(Duration::from_secs(5)))
        })
        .collect();
    // Os dois já pediram quando a captura chega: espera o pedido e mais um instante para o segundo entrar.
    esperar_pedido(&quadros);
    std::thread::sleep(Duration::from_millis(100));
    quadros.entregar(camera::quadro_de_teste(7));
    let recebidos: Vec<_> = espera
        .into_iter()
        .map(|t| t.join().unwrap().expect("um quadro"))
        .collect();
    assert!(
        Arc::ptr_eq(&recebidos[0], &recebidos[1]),
        "os dois recebem a mesma captura, e não duas cópias"
    );
    assert_eq!(quadros.copias(), 1);
}

#[test]
fn sem_pedido_nao_copia() {
    let quadros = Quadros::default();
    let parar = fonte(&quadros);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        quadros.copias(),
        0,
        "a fonte copiou quadro sem ninguém pedir"
    );
    assert!(quadros.proximo_em(Duration::from_secs(5)).is_some());
    assert_eq!(quadros.copias(), 1, "um pedido, uma cópia");
    parar.store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(camera::bytes_nv12(TESTE_LARGURA, TESTE_ALTURA), 55_296);
}
