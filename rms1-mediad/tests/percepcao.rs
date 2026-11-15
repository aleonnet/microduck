//! A percepção pela estação: os critérios 16, 17 e 19 do plano. A estação daqui é de mentira: anuncia-se em 127.0.0.1,
//! aceita a conexão dos quadros, guarda o que chega e manda as entregas que o teste pede.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rms1_mediad::camera;
use rms1_mediad::estacao::{ANUNCIO_VALE, Cabecalho, Tipo};
use rms1_mediad::percepcao::{self, Config, Estado};
use rms1_mediad::quadros::Quadros;

/// A resposta do detector, como a estação a manda: o total, quantas vão, e cinco números por caixa, na moldura de 320.
fn carga(caixas: &[[f32; 5]]) -> Vec<u8> {
    let mut c = Vec::with_capacity(1008);
    c.extend((caixas.len() as i32).to_le_bytes());
    c.extend((caixas.len() as i32).to_le_bytes());
    for numero in caixas.iter().flatten() {
        c.extend(numero.to_le_bytes());
    }
    c.resize(1008, 0);
    c
}

#[test]
fn caixas_da_estacao_viram_media_detections() {
    // Duas caixas sobre o mesmo pato (fica a de nota maior), uma longe, e uma abaixo da nota.
    let resposta = carga(&[
        [160.0, 160.0, 40.0, 20.0, 0.80],
        [162.0, 160.0, 40.0, 20.0, 0.90],
        [40.0, 100.0, 20.0, 20.0, 0.50],
        [250.0, 200.0, 20.0, 20.0, 0.30],
    ]);
    let achadas = percepcao::deteccoes(&resposta, 1280, 720, 0.35, 0.5).unwrap();
    assert_eq!(achadas.len(), 2, "{achadas:?}");
    // De volta ao quadro de 1280 por 720: tira a faixa de 70 linhas da moldura e multiplica por 4.
    assert_eq!(achadas[0].box_, [568.0, 320.0, 728.0, 400.0]);
    assert_eq!(achadas[0].score, 0.90);
    assert_eq!(achadas[1].box_, [120.0, 80.0, 200.0, 160.0]);
    // A nota é a que vem de fora: com 0.25, a quarta caixa entra.
    assert_eq!(
        percepcao::deteccoes(&resposta, 1280, 720, 0.25, 0.5)
            .unwrap()
            .len(),
        3
    );
    // Num quadro de outro tamanho, a volta é para o tamanho dele.
    let pequeno = percepcao::deteccoes(&resposta, 256, 144, 0.35, 0.5).unwrap();
    assert_eq!(pequeno[1].box_, [24.0, 16.0, 40.0, 32.0]);

    // O formato é o do pato (`mediad/src/detect.rs`, `notification`).
    let v: serde_json::Value =
        serde_json::from_str(&percepcao::notificacao(&achadas, 1280, 720, 45.37)).unwrap();
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["method"], "media.detections");
    assert!(v.get("id").is_none(), "é notificação, sem id");
    let p = &v["params"];
    assert_eq!(
        (p["width"].as_u64(), p["height"].as_u64()),
        (Some(1280), Some(720))
    );
    assert_eq!(p["took_ms"].as_f64(), Some(45.4));
    assert_eq!(p["boxes"].as_array().unwrap().len(), 2);
    assert_eq!(
        p["boxes"][0],
        serde_json::json!({"x0": 568.0, "y0": 320.0, "x1": 728.0, "y1": 400.0, "score": 0.9})
    );
    // Sem caixa, a notificação sai assim mesmo, com a lista vazia.
    let vazia: serde_json::Value =
        serde_json::from_str(&percepcao::notificacao(&[], 1280, 720, 45.0)).unwrap();
    assert_eq!(vazia["params"]["boxes"], serde_json::json!([]));
    assert_eq!(
        percepcao::deteccoes(&carga(&[]), 1280, 720, 0.35, 0.5).unwrap(),
        vec![]
    );

    // Resposta de outro tamanho, ou que diz mais caixas do que cabem, é recusada com o motivo.
    assert!(
        percepcao::deteccoes(&resposta[..1007], 1280, 720, 0.35, 0.5)
            .unwrap_err()
            .contains("1007 bytes")
    );
    let mut mentira = resposta.clone();
    mentira[4..8].copy_from_slice(&51i32.to_le_bytes());
    assert!(
        percepcao::deteccoes(&mentira, 1280, 720, 0.35, 0.5)
            .unwrap_err()
            .contains("51 caixas")
    );
}

/// O que a estação de mentira recebeu: o cabeçalho, o tamanho da carga, a carga se for pequena, e quando.
struct Recebida {
    cab: Cabecalho,
    carga: Vec<u8>,
    quando: Instant,
}

struct Estacao {
    anunciando: Arc<AtomicBool>,
    recebidas: Receiver<Recebida>,
    /// A conexão dos quadros, para o teste mandar entregas.
    conexao: Arc<Mutex<Option<TcpStream>>>,
}

impl Estacao {
    /// Sobe a estação de mentira anunciando `funcoes` e `entradas` para a porta `porta_anuncio`, de 127.0.0.1.
    fn subir(porta_anuncio: u16, entradas: &str, funcoes: &str) -> Self {
        let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
        let anuncio = format!(
            r#"{{"rms1_hailo":2,"quadros":{},"painel":8080,"entradas":{entradas},"funcoes":{funcoes}}}"#,
            tcp.local_addr().unwrap().port()
        );
        let anunciando = Arc::new(AtomicBool::new(true));
        let a = anunciando.clone();
        std::thread::spawn(move || {
            let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
            loop {
                if a.load(Ordering::Relaxed) {
                    let _ = udp.send_to(anuncio.as_bytes(), ("127.0.0.1", porta_anuncio));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let (tx, recebidas) = mpsc::channel();
        let conexao = Arc::new(Mutex::new(None));
        let guardada = conexao.clone();
        std::thread::spawn(move || {
            for c in tcp.incoming() {
                let Ok(mut c) = c else { return };
                *guardada.lock().unwrap() = c.try_clone().ok();
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let mut cab = [0u8; Cabecalho::BYTES];
                    while c.read_exact(&mut cab).is_ok() {
                        let cab = Cabecalho::ler(&cab).expect("o S1 manda cabeçalho válido");
                        let mut carga = vec![0u8; cab.tamanho as usize];
                        if c.read_exact(&mut carga).is_err() {
                            return;
                        }
                        if tx
                            .send(Recebida {
                                cab,
                                carga,
                                quando: Instant::now(),
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });
        Self {
            anunciando,
            recebidas,
            conexao,
        }
    }

    fn entregar(&self, canal: u16, carga: &[u8]) {
        let mut c = self.conexao.lock().unwrap();
        let c = c.as_mut().expect("o S1 ligado");
        let cab = Cabecalho {
            tipo: Tipo::Entrega,
            canal,
            tamanho: carga.len() as u32,
        };
        c.write_all(&cab.bytes()).unwrap();
        c.write_all(carga).unwrap();
    }

    /// Tudo o que chega em `por`.
    fn colher(&self, por: Duration) -> Vec<Recebida> {
        let fim = Instant::now() + por;
        let mut tudo = Vec::new();
        while let Some(resta) = fim.checked_duration_since(Instant::now()) {
            match self.recebidas.recv_timeout(resta) {
                Ok(r) => tudo.push(r),
                Err(_) => break,
            }
        }
        tudo
    }
}

struct Percepcao {
    estado: Arc<Mutex<Estado>>,
    publicadas: Arc<Mutex<Vec<String>>>,
    parar: Arc<AtomicBool>,
}

impl Drop for Percepcao {
    fn drop(&mut self) {
        self.parar.store(true, Ordering::Relaxed);
    }
}

/// Uma porta UDP livre, para cada teste ter o seu anúncio.
fn porta_livre() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Sobe a percepção com a câmera de teste (256 por 144) e o teto de ritmo dado.
fn subir_percepcao(porta_anuncio: u16, ritmo_max: f64, anuncio_vale: Duration) -> Percepcao {
    let quadros = Quadros::default();
    let parar = Arc::new(AtomicBool::new(false));
    let (q, p) = (quadros.clone(), parar.clone());
    std::thread::spawn(move || camera::rodar_teste(q, Duration::from_millis(5), p));
    let estado = Arc::new(Mutex::new(Estado::default()));
    let publicadas = Arc::new(Mutex::new(Vec::new()));
    let (e, pubs, p) = (estado.clone(), publicadas.clone(), parar.clone());
    std::thread::spawn(move || {
        let cfg = Config {
            porta_anuncio,
            ritmo_max,
            anuncio_vale,
        };
        percepcao::rodar(
            cfg,
            quadros,
            move |linha| pubs.lock().unwrap().push(linha),
            e,
            p,
        )
        .unwrap();
    });
    Percepcao {
        estado,
        publicadas,
        parar,
    }
}

const DUAS_ENTRADAS: &str = r#"[{"n":1,"id":"quadro_continuo","formato":"rgb_640x360","ritmo":10},{"n":2,"id":"audio","formato":"pcm16_16k","ritmo":50}]"#;
const DUAS_FUNCOES: &str = r#"[{"n":1,"id":"detector","entrega":"caixas","leitor":"nanodet_v8","publica":"media.detections","nota":0.35,"iou":0.5},{"n":2,"id":"vigia","entrega":"caixas","leitor":"nanodet_v8","nota":0.35,"iou":0.5}]"#;

#[test]
fn segue_o_anuncio_da_estacao() {
    assert_eq!(
        ANUNCIO_VALE,
        Duration::from_secs(3),
        "sem anúncio por 3 s, a estação saiu"
    );
    let porta = porta_livre();
    let p = subir_percepcao(porta, 22.0, Duration::from_millis(600));
    // Sem estação, nada é mandado, e o estado diz por quê.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        p.estado.lock().unwrap().idle_reason.as_deref(),
        Some("nenhuma estação se anunciou na rede")
    );

    let estacao = Estacao::subir(porta, DUAS_ENTRADAS, DUAS_FUNCOES);
    let primeira = estacao
        .recebidas
        .recv_timeout(Duration::from_secs(3))
        .expect("o primeiro quadro");
    // Só o quadro é mandado: a entrada de áudio, que este S1 não sabe mandar, não aparece.
    let chegadas = estacao.colher(Duration::from_secs(2));
    let quadros: Vec<&Recebida> = chegadas
        .iter()
        .filter(|r| r.cab.tipo == Tipo::Entrada)
        .collect();
    assert!(
        chegadas
            .iter()
            .all(|r| matches!(r.cab.tipo, Tipo::Entrada | Tipo::Medida))
    );
    assert!(
        quadros
            .iter()
            .all(|r| r.cab.canal == 1 && r.carga.len() == 691_200),
        "só o quadro contínuo, RGB 640 por 360"
    );
    assert_eq!(primeira.carga.len(), 691_200);
    // No ritmo anunciado, 10 por segundo, e não no teto de 22 nem no da câmera: em 2 s, de 18 a 21 quadros.
    assert!(
        (18..=21).contains(&quadros.len()),
        "{} quadros em 2 s",
        quadros.len()
    );
    let menor = quadros
        .windows(2)
        .map(|w| w[1].quando - w[0].quando)
        .min()
        .unwrap();
    assert!(
        menor >= Duration::from_millis(60),
        "dois quadros a {menor:?} um do outro: rajada"
    );
    let e = p.estado.lock().unwrap().clone();
    assert_eq!(
        (e.idle_reason, e.rate, e.station.as_deref()),
        (None, Some(10.0), Some("127.0.0.1"))
    );

    // As duas funções entregam; só a que pede é publicada, e a outra é contada.
    let resposta = carga(&[
        [160.0, 160.0, 40.0, 20.0, 0.40],
        [40.0, 100.0, 20.0, 20.0, 0.30],
    ]);
    estacao.entregar(1, &resposta);
    estacao.entregar(2, &resposta);
    estacao.entregar(1, &carga(&[]));
    std::thread::sleep(Duration::from_millis(300));
    let publicadas = p.publicadas.lock().unwrap().clone();
    assert_eq!(publicadas.len(), 2, "{publicadas:?}");
    let v: serde_json::Value = serde_json::from_str(&publicadas[0]).unwrap();
    // A nota é a do anúncio (0.35): das duas candidatas, entra a de 0.40. E as caixas voltam ao quadro da câmera.
    assert_eq!(
        (
            v["params"]["width"].as_u64(),
            v["params"]["height"].as_u64()
        ),
        (Some(256), Some(144))
    );
    assert_eq!(
        v["params"]["boxes"],
        serde_json::json!([{"x0": 112.0, "y0": 64.0, "x1": 144.0, "y1": 80.0, "score": 0.4}])
    );
    let vazia: serde_json::Value = serde_json::from_str(&publicadas[1]).unwrap();
    assert_eq!(vazia["params"]["boxes"], serde_json::json!([]));
    let e = p.estado.lock().unwrap().clone();
    assert_eq!(
        (
            e.deliveries.get("detector"),
            e.deliveries.get("vigia"),
            e.errors
        ),
        (Some(&2), Some(&1), 0)
    );
    assert!(e.round_trip_ms.is_some());

    // A estação para de se anunciar: passado o prazo, o S1 para de mandar.
    estacao.anunciando.store(false, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(900));
    let _ = estacao.colher(Duration::from_millis(1));
    assert!(
        estacao.colher(Duration::from_millis(500)).is_empty(),
        "mandou quadro sem anúncio"
    );
    assert_eq!(
        p.estado.lock().unwrap().idle_reason.as_deref(),
        Some("nenhuma estação se anunciou na rede")
    );
    // E volta sozinho quando ela volta.
    estacao.anunciando.store(true, Ordering::Relaxed);
    assert!(
        estacao
            .recebidas
            .recv_timeout(Duration::from_secs(3))
            .is_ok(),
        "não voltou a mandar"
    );
}

#[test]
fn o_teto_de_ritmo_vale_sobre_o_anuncio() {
    let porta = porta_livre();
    let _p = subir_percepcao(porta, 5.0, Duration::from_secs(3));
    let estacao = Estacao::subir(porta, DUAS_ENTRADAS, DUAS_FUNCOES);
    estacao
        .recebidas
        .recv_timeout(Duration::from_secs(3))
        .expect("o primeiro quadro");
    let n = estacao
        .colher(Duration::from_secs(2))
        .iter()
        .filter(|r| r.cab.tipo == Tipo::Entrada)
        .count();
    assert!(
        (9..=11).contains(&n),
        "{n} quadros em 2 s, com o teto em 5 por segundo"
    );
}

#[test]
fn manda_a_medida_a_cada_segundo() {
    let porta = porta_livre();
    let _p = subir_percepcao(porta, 22.0, Duration::from_secs(3));
    let estacao = Estacao::subir(porta, DUAS_ENTRADAS, DUAS_FUNCOES);
    estacao
        .recebidas
        .recv_timeout(Duration::from_secs(3))
        .expect("o primeiro quadro");
    estacao.entregar(1, &carga(&[]));
    let chegadas = estacao.colher(Duration::from_millis(3500));
    let medidas: Vec<&Recebida> = chegadas
        .iter()
        .filter(|r| r.cab.tipo == Tipo::Medida)
        .collect();
    let quadros = chegadas
        .iter()
        .filter(|r| r.cab.tipo == Tipo::Entrada)
        .count();
    // Uma por segundo, e não uma por quadro.
    assert!(
        (3..=4).contains(&medidas.len()),
        "{} medidas em 3,5 s, com {quadros} quadros",
        medidas.len()
    );
    let afastadas = medidas
        .windows(2)
        .all(|w| w[1].quando - w[0].quando >= Duration::from_millis(900));
    assert!(afastadas, "duas medidas a menos de um segundo uma da outra");
    let ultima: serde_json::Value = serde_json::from_slice(&medidas.last().unwrap().carga).unwrap();
    assert_eq!(medidas[0].cab.canal, 0);
    let por_s = ultima["quadros_por_s"].as_f64().unwrap();
    assert!(
        (8.0..=11.0).contains(&por_s),
        "{por_s} quadros por segundo medidos, a 10 anunciados"
    );
    assert!(
        ultima["ida_e_volta_ms"].as_f64().is_some_and(|ms| ms > 0.0),
        "{ultima}"
    );
}

#[test]
fn com_duas_estacoes_fica_com_a_que_ja_usa() {
    let porta = porta_livre();
    let _p = subir_percepcao(porta, 22.0, Duration::from_millis(600));
    let primeira = Estacao::subir(porta, DUAS_ENTRADAS, DUAS_FUNCOES);
    primeira
        .recebidas
        .recv_timeout(Duration::from_secs(3))
        .expect("o primeiro quadro, na primeira");
    // Chega outra estação à rede: o S1 não troca, nem divide os quadros.
    let segunda = Estacao::subir(porta, DUAS_ENTRADAS, DUAS_FUNCOES);
    let na_primeira = primeira
        .colher(Duration::from_millis(1500))
        .iter()
        .filter(|r| r.cab.tipo == Tipo::Entrada)
        .count();
    assert!(
        (13..=16).contains(&na_primeira),
        "{na_primeira} quadros na primeira em 1,5 s: o S1 largou a estação que usava"
    );
    assert!(
        segunda.colher(Duration::from_millis(1)).is_empty(),
        "a segunda estação recebeu quadro"
    );
    // A primeira sai da rede: passado o prazo, o S1 vai para a que ficou.
    primeira.anunciando.store(false, Ordering::Relaxed);
    assert!(
        segunda
            .recebidas
            .recv_timeout(Duration::from_secs(4))
            .is_ok(),
        "não foi para a estação que ficou"
    );
}

/// A estação que recusa o robô (ele não está entre os permitidos) responde com erro e fecha. O S1 volta a tentar, e cada
/// vez espera mais: no robô de verdade, em 2026-11-15, ele religava várias vezes por segundo até ser permitido no painel.
#[test]
fn estacao_que_recusa_nao_e_martelada() {
    let porta = porta_livre();
    let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
    let anuncio = format!(
        r#"{{"rms1_hailo":2,"quadros":{},"painel":8080,"entradas":{DUAS_ENTRADAS},"funcoes":{DUAS_FUNCOES}}}"#,
        tcp.local_addr().unwrap().port()
    );
    std::thread::spawn(move || {
        let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        loop {
            let _ = udp.send_to(anuncio.as_bytes(), ("127.0.0.1", porta));
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    let ligacoes = Arc::new(Mutex::new(Vec::new()));
    let l = ligacoes.clone();
    std::thread::spawn(move || {
        for c in tcp.incoming() {
            let Ok(mut c) = c else { return };
            l.lock().unwrap().push(Instant::now());
            // Lê a primeira mensagem, recusa e fecha, como o serviço da estação faz com um robô não permitido.
            let mut cab = [0u8; Cabecalho::BYTES];
            if c.read_exact(&mut cab).is_ok() {
                let lido = Cabecalho::ler(&cab).expect("cabeçalho válido");
                let mut carga = vec![0u8; lido.tamanho as usize];
                let _ = c.read_exact(&mut carga);
                let motivo = "o robô 127.0.0.1 não está entre os permitidos".as_bytes();
                let erro = Cabecalho {
                    tipo: Tipo::Erro,
                    canal: 0,
                    tamanho: motivo.len() as u32,
                };
                let _ = c.write_all(&erro.bytes());
                let _ = c.write_all(motivo);
            }
        }
    });
    let p = subir_percepcao(porta, 22.0, Duration::from_millis(600));
    std::thread::sleep(Duration::from_secs(4));
    let n = ligacoes.lock().unwrap().len();
    // Com a espera dobrando de 100 ms até 2 s, cabem umas seis ligações em 4 s; sem ela, dezenas.
    assert!((2..=9).contains(&n), "{n} ligações em 4 s");
    assert!(p.estado.lock().unwrap().errors >= 1, "a recusa é contada");
}

/// A entrega do leitor `caixas_padrao_hailo` (um detector do catálogo, com o filtro de caixas feito no Hailo): seis números
/// por caixa, com a classe. O S1 filtra pela nota da função, volta ao quadro da câmera e publica com o nome da classe.
#[test]
fn caixas_com_classe_viram_media_detections_com_o_nome() {
    // As três caixas que o Hailo de mentira do laboratório devolve (`hailo_tubo_falso.py`, `CAIXAS_COM_CLASSE`).
    let caixas: [[f32; 6]; 3] = [
        [160.0, 160.0, 80.0, 120.0, 0.91, 0.0],
        [64.0, 200.0, 32.0, 32.0, 0.55, 32.0],
        [250.0, 150.0, 40.0, 60.0, 0.12, 56.0],
    ];
    let mut entrega = vec![0u8; 1208];
    entrega[..4].copy_from_slice(&3i32.to_le_bytes());
    entrega[4..8].copy_from_slice(&3i32.to_le_bytes());
    for (i, c) in caixas.iter().enumerate() {
        for (j, v) in c.iter().enumerate() {
            let em = 8 + (i * 6 + j) * 4;
            entrega[em..em + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    // No quadro da câmera do robô, 1280 por 720: a moldura tira 70 linhas, e o quadro é quatro vezes o de 320 por 180.
    let achadas = percepcao::deteccoes_com_classe(&entrega, 1280, 720, 0.35).unwrap();
    assert_eq!(
        achadas.len(),
        2,
        "a cadeira, de nota 0,12, fica abaixo da nota"
    );
    assert_eq!(achadas[0].0.box_, [480.0, 120.0, 800.0, 600.0]);
    assert_eq!((achadas[0].1, achadas[1].1), (0, 32));
    assert_eq!(achadas[1].0.box_, [192.0, 456.0, 320.0, 584.0]);
    let v: serde_json::Value = serde_json::from_str(&percepcao::notificacao_com_classe(
        &achadas, 1280, 720, 31.0,
    ))
    .unwrap();
    assert_eq!(v["method"], "media.detections");
    assert_eq!(
        v["params"]["boxes"],
        serde_json::json!([
            {"x0": 480.0, "y0": 120.0, "x1": 800.0, "y1": 600.0, "score": 0.91, "class": 0, "label": "person"},
            {"x0": 192.0, "y0": 456.0, "x1": 320.0, "y1": 584.0, "score": 0.55, "class": 32, "label": "sports ball"}
        ])
    );
    assert_eq!(
        (percepcao::CLASSES_COCO[56], percepcao::CLASSES_COCO[79]),
        ("chair", "toothbrush")
    );
    // Tamanho errado e contagem absurda são recusados com o motivo.
    assert!(
        percepcao::deteccoes_com_classe(&entrega[..1008], 1280, 720, 0.35)
            .unwrap_err()
            .contains("1008")
    );
    entrega[4..8].copy_from_slice(&51i32.to_le_bytes());
    assert!(
        percepcao::deteccoes_com_classe(&entrega, 1280, 720, 0.35)
            .unwrap_err()
            .contains("51")
    );
}
