//! A câmera do simulador: os critérios 48 e 49 do plano. O simulador daqui é de mentira: um servidor TCP que conta os
//! pedidos e responde o que o teste manda.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rms1_mediad::camera::{self, NV12, S1_ALTURA, S1_LARGURA, bytes_nv12};
use rms1_mediad::quadros::Quadros;

/// O quadro de número `n`, do tamanho da câmera do robô: reconhecível pelos primeiros bytes.
fn quadro(n: u8) -> Vec<u8> {
    let mut q = vec![n; bytes_nv12(S1_LARGURA, S1_ALTURA)];
    q[..4].copy_from_slice(&[0xCA, 0xFE, n, n.wrapping_mul(3)]);
    q
}

struct Simulador {
    endereco: String,
    pedidos: Arc<AtomicUsize>,
    conexoes: Arc<AtomicUsize>,
}

/// Sobe o simulador de mentira. A cada pedido (4 bytes), responde com a próxima resposta da lista: os bytes que ela tiver,
/// com o tamanho na frente. Acabada a lista, repete a última.
fn simulador(respostas: Vec<Vec<u8>>) -> Simulador {
    let escuta = TcpListener::bind("127.0.0.1:0").unwrap();
    let endereco = escuta.local_addr().unwrap().to_string();
    let (pedidos, conexoes) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (p, c) = (pedidos.clone(), conexoes.clone());
    std::thread::spawn(move || {
        for conexao in escuta.incoming() {
            let Ok(mut conexao) = conexao else { return };
            c.fetch_add(1, Ordering::SeqCst);
            let mut pedido = [0u8; 4];
            while conexao.read_exact(&mut pedido).is_ok() {
                assert_eq!(
                    pedido,
                    [0, 0, 0, 0],
                    "o pedido de quadro são 4 bytes de zero"
                );
                let n = p.fetch_add(1, Ordering::SeqCst);
                let resposta = &respostas[n.min(respostas.len() - 1)];
                if conexao
                    .write_all(&(resposta.len() as u32).to_le_bytes())
                    .and_then(|()| conexao.write_all(resposta))
                    .is_err()
                {
                    break;
                }
            }
        }
    });
    Simulador {
        endereco,
        pedidos,
        conexoes,
    }
}

struct Camera {
    quadros: Quadros,
    ditos: Arc<Mutex<Vec<Option<String>>>>,
    parar: Arc<AtomicBool>,
}

impl Drop for Camera {
    fn drop(&mut self) {
        self.parar.store(true, Ordering::Relaxed);
    }
}

fn camera(endereco: &str) -> Camera {
    let quadros = Quadros::default();
    let parar = Arc::new(AtomicBool::new(false));
    let ditos = Arc::new(Mutex::new(Vec::new()));
    let (q, p, d, e) = (
        quadros.clone(),
        parar.clone(),
        ditos.clone(),
        endereco.to_owned(),
    );
    std::thread::spawn(move || {
        camera::rodar_simulador(
            q,
            &e,
            Duration::from_millis(100),
            move |motivo| d.lock().unwrap().push(motivo),
            p,
        )
    });
    Camera {
        quadros,
        ditos,
        parar,
    }
}

#[test]
fn camera_do_simulador_entrega_o_quadro_pedido() {
    let sim = simulador(vec![quadro(1), quadro(2)]);
    let c = camera(&sim.endereco);
    // Sem ninguém pedindo, o simulador não é incomodado: ele só desenha o quadro que alguém quer.
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        sim.pedidos.load(Ordering::SeqCst),
        0,
        "pediu quadro sem ninguém querer"
    );
    assert_eq!(c.quadros.copias(), 0);

    let q = c.quadros.proximo().expect("o quadro pedido");
    assert_eq!((q.largura, q.altura, q.formato), (1280, 720, NV12));
    assert_eq!(q.dados.len(), 1_382_400);
    assert!(
        q.dados == quadro(1),
        "os bytes do quadro mudaram no caminho"
    );
    assert_eq!(
        sim.pedidos.load(Ordering::SeqCst),
        1,
        "um pedido, um quadro desenhado"
    );

    // O próximo pedido é respondido por outro quadro, e não pelo guardado.
    let q = c.quadros.proximo().expect("o segundo quadro");
    assert!(q.dados == quadro(2));
    assert_eq!(
        (sim.pedidos.load(Ordering::SeqCst), c.quadros.copias()),
        (2, 2)
    );
    assert_eq!(
        c.ditos.lock().unwrap().last(),
        Some(&None),
        "com o quadro chegando, a câmera não tem do que se queixar"
    );
}

#[test]
fn camera_do_simulador_recusa_tamanho_errado_e_reconecta() {
    // A primeira resposta tem o tamanho errado; as seguintes, o certo.
    let sim = simulador(vec![vec![7u8; 100], quadro(5)]);
    let c = camera(&sim.endereco);
    assert!(
        c.quadros.proximo().is_none(),
        "um quadro de tamanho errado foi entregue"
    );
    let dito = c
        .ditos
        .lock()
        .unwrap()
        .iter()
        .flatten()
        .next_back()
        .cloned()
        .expect("o motivo da recusa");
    assert!(dito.contains("100") && dito.contains("1382400"), "{dito}");
    // A conexão é refeita sozinha, e o pedido seguinte é atendido.
    let q = c
        .quadros
        .proximo_em(Duration::from_secs(3))
        .expect("o quadro depois de reconectar");
    assert!(q.dados == quadro(5));
    assert_eq!(
        sim.conexoes.load(Ordering::SeqCst),
        2,
        "a conexão não foi refeita"
    );
}

#[test]
fn sem_pagina_aberta_a_camera_diz_e_nao_refaz_a_conexao() {
    // O serviço local está no ar, mas nenhuma página desenha: ele responde com um quadro de tamanho zero. Depois, a página abre.
    let sim = simulador(vec![vec![], vec![], quadro(3)]);
    let c = camera(&sim.endereco);
    assert!(c.quadros.proximo().is_none());
    let dito = c
        .ditos
        .lock()
        .unwrap()
        .iter()
        .flatten()
        .next_back()
        .cloned()
        .expect("o motivo");
    assert!(dito.contains("nenhuma página"), "{dito}");
    assert!(c.quadros.proximo().is_none());
    let q = c.quadros.proximo().expect("o quadro, com a página aberta");
    assert!(q.dados == quadro(3));
    assert_eq!(
        sim.conexoes.load(Ordering::SeqCst),
        1,
        "a conexão foi refeita sem precisar"
    );
    assert_eq!(c.ditos.lock().unwrap().last(), Some(&None));
}

#[test]
fn sem_simulador_a_camera_diz_por_que_e_segue_tentando() {
    // Uma porta em que ninguém ouve.
    let livre = TcpListener::bind("127.0.0.1:0").unwrap();
    let endereco = livre.local_addr().unwrap().to_string();
    drop(livre);
    let c = camera(&endereco);
    assert!(c.quadros.proximo().is_none());
    let dito = c
        .ditos
        .lock()
        .unwrap()
        .iter()
        .flatten()
        .next_back()
        .cloned()
        .expect("o motivo");
    assert!(dito.contains(&endereco), "{dito}");
    // O simulador sobe depois, no mesmo endereço: a câmera o acha sozinha.
    let escuta = TcpListener::bind(&endereco).unwrap();
    std::thread::spawn(move || {
        let (mut conexao, _) = escuta.accept().unwrap();
        let mut pedido = [0u8; 4];
        while conexao.read_exact(&mut pedido).is_ok() {
            let q = quadro(9);
            let _ = conexao
                .write_all(&(q.len() as u32).to_le_bytes())
                .and_then(|()| conexao.write_all(&q));
        }
    });
    let q = c
        .quadros
        .proximo_em(Duration::from_secs(3))
        .expect("o quadro, com o simulador de volta");
    assert!(q.dados == quadro(9));
}
