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
        Self::subir_com(porta_anuncio, entradas, funcoes, "")
    }

    /// O mesmo, com mais campos no anúncio (`extra` começa pela vírgula).
    fn subir_com(porta_anuncio: u16, entradas: &str, funcoes: &str, extra: &str) -> Self {
        let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
        let anuncio = format!(
            r#"{{"rms1_hailo":2,"quadros":{},"painel":8080,"entradas":{entradas},"funcoes":{funcoes}{extra}}}"#,
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

    /// A estação marca o quadro `n` atendido.
    fn atender(&self, n: u16) {
        let mut c = self.conexao.lock().unwrap();
        let cab = Cabecalho {
            tipo: Tipo::Atendido,
            canal: n,
            tamanho: 0,
        };
        c.as_mut()
            .expect("o S1 ligado")
            .write_all(&cab.bytes())
            .unwrap();
    }

    /// A estação recusa uma mensagem do canal dado, com o motivo.
    fn recusar(&self, canal: u16, motivo: &str) {
        let mut c = self.conexao.lock().unwrap();
        let c = c.as_mut().expect("o S1 ligado");
        let cab = Cabecalho {
            tipo: Tipo::Erro,
            canal,
            tamanho: motivo.len() as u32,
        };
        c.write_all(&cab.bytes()).unwrap();
        c.write_all(motivo.as_bytes()).unwrap();
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

/// Sobe a percepção com a câmera de teste (256 por 144) e o teto de ritmo dado, sem codificador.
fn subir_percepcao(porta_anuncio: u16, ritmo_max: f64, anuncio_vale: Duration) -> Percepcao {
    subir_percepcao_com(porta_anuncio, ritmo_max, anuncio_vale, None)
}

/// O mesmo, com o codificador de hardware do robô (ou o de mentira) no socket dado.
fn subir_percepcao_com(
    porta_anuncio: u16,
    ritmo_max: f64,
    anuncio_vale: Duration,
    venc: Option<std::path::PathBuf>,
) -> Percepcao {
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
            venc: venc.map(|caminho| Arc::new(Mutex::new(rms1_mediad::venc::Venc::novo(caminho)))),
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

// ---- o quadro codificado pelo chip do S1 (adendo 4, 2026-11-16) ------------------------------------------------------------

/// O `rms1-venc` de mentira: fala o protocolo do socket e anota cada pedido (comando, parâmetro, bytes da carga). O quadro
/// em H.264 volta começando pelo código de início, e o JPEG por `FF D8`, como os de verdade.
struct VencFalso {
    caminho: std::path::PathBuf,
    pedidos: Arc<Mutex<Vec<(u8, u32, usize)>>>,
}

impl VencFalso {
    fn subir() -> Self {
        use std::os::unix::net::UnixListener;
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let caminho = std::env::temp_dir().join(format!(
            "rms1-venc-{}-{}.sock",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&caminho);
        let ouvinte = UnixListener::bind(&caminho).unwrap();
        let pedidos = Arc::new(Mutex::new(Vec::new()));
        let anotados = pedidos.clone();
        std::thread::spawn(move || {
            for c in ouvinte.incoming() {
                let Ok(mut c) = c else { return };
                let mut cab = [0u8; 9];
                while c.read_exact(&mut cab).is_ok() {
                    let param = u32::from_le_bytes([cab[1], cab[2], cab[3], cab[4]]);
                    let tamanho = u32::from_le_bytes([cab[5], cab[6], cab[7], cab[8]]) as usize;
                    let mut carga = vec![0u8; tamanho];
                    if c.read_exact(&mut carga).is_err() {
                        break;
                    }
                    anotados.lock().unwrap().push((cab[0], param, tamanho));
                    let resposta: Vec<u8> = match cab[0] {
                        1 => [&[0u8, 0, 0, 1][..], b"quadro em h264"].concat(),
                        2 => [&[0xffu8, 0xd8][..], b"quadro em jpeg"].concat(),
                        _ => Vec::new(),
                    };
                    let mut r = vec![0u8];
                    r.extend((resposta.len() as u32).to_le_bytes());
                    r.extend(resposta);
                    if c.write_all(&r).is_err() {
                        break;
                    }
                }
            }
        });
        Self { caminho, pedidos }
    }

    fn pedidos(&self) -> Vec<(u8, u32, usize)> {
        self.pedidos.lock().unwrap().clone()
    }
}

const SO_O_DETECTOR: &str = r#"[{"n":1,"id":"detector","entrega":"caixas","leitor":"nanodet_v8","publica":"media.detections","nota":0.35,"iou":0.5}]"#;

#[test]
fn com_o_codificador_o_quadro_vai_em_h264_e_o_fluxo_comeca_com_a_conexao() {
    let porta = porta_livre();
    let venc = VencFalso::subir();
    let estacao = Estacao::subir(
        porta,
        r#"[{"n":1,"id":"quadro_continuo","formato":"rgb_640x360","prefere":"h264","qualidade":0,"ritmo":10}]"#,
        SO_O_DETECTOR,
    );
    let p = subir_percepcao_com(
        porta,
        22.0,
        Duration::from_secs(3),
        Some(venc.caminho.clone()),
    );
    let quadros: Vec<Recebida> = estacao
        .colher(Duration::from_millis(900))
        .into_iter()
        .filter(|r| r.cab.tipo != Tipo::Medida)
        .collect();
    assert!(quadros.len() >= 4, "{} quadros em 0,9 s", quadros.len());
    assert!(
        quadros.iter().all(|r| r.cab.tipo == Tipo::EntradaH264
            && r.cab.canal == 1
            && r.carga.starts_with(&[0, 0, 0, 1])),
        "todo quadro vai no tipo de H.264, na entrada do anúncio, como o codificador o fez"
    );
    let pedidos = venc.pedidos();
    // O primeiro pedido é o fluxo novo, com a taxa (0, o máximo do chip) e o ritmo (10) do anúncio; depois, só quadros.
    assert_eq!(
        pedidos[0],
        (3, 10 << 16, 0),
        "{:?}",
        &pedidos[..3.min(pedidos.len())]
    );
    assert!(
        pedidos[1..].iter().all(|x| *x == (1, 0, 640 * 360 * 3 / 2)),
        "{:?}",
        &pedidos[..4.min(pedidos.len())]
    );
    let e = p.estado.lock().unwrap().clone();
    assert_eq!(
        (e.format.as_deref(), e.encoder_problem),
        (Some("h264"), None)
    );
}

#[test]
fn em_jpeg_o_nivel_e_o_do_anuncio_e_nao_ha_fluxo() {
    let porta = porta_livre();
    let venc = VencFalso::subir();
    let estacao = Estacao::subir(
        porta,
        r#"[{"n":1,"id":"quadro_continuo","formato":"rgb_640x360","prefere":"jpeg","qualidade":9,"ritmo":10}]"#,
        SO_O_DETECTOR,
    );
    let p = subir_percepcao_com(
        porta,
        22.0,
        Duration::from_secs(3),
        Some(venc.caminho.clone()),
    );
    let quadros: Vec<Recebida> = estacao
        .colher(Duration::from_millis(700))
        .into_iter()
        .filter(|r| r.cab.tipo != Tipo::Medida)
        .collect();
    assert!(quadros.len() >= 3);
    assert!(
        quadros
            .iter()
            .all(|r| r.cab.tipo == Tipo::EntradaJpeg && r.carga.starts_with(&[0xff, 0xd8]))
    );
    assert!(
        venc.pedidos()
            .iter()
            .all(|x| *x == (2, 9, 640 * 360 * 3 / 2)),
        "{:?}",
        venc.pedidos().first()
    );
    assert_eq!(p.estado.lock().unwrap().format.as_deref(), Some("jpeg"));
}

#[test]
fn sem_o_codificador_no_ar_o_quadro_vai_cru_e_o_estado_diz_o_motivo() {
    let porta = porta_livre();
    let estacao = Estacao::subir(
        porta,
        r#"[{"n":1,"id":"quadro_continuo","formato":"rgb_640x360","prefere":"h264","qualidade":0,"ritmo":10}]"#,
        SO_O_DETECTOR,
    );
    // O arquivo dá o socket, e ninguém o atende: o robô sem o rms1-venc no ar.
    let ausente =
        std::env::temp_dir().join(format!("rms1-venc-ausente-{}.sock", std::process::id()));
    let p = subir_percepcao_com(porta, 22.0, Duration::from_secs(3), Some(ausente));
    let quadros: Vec<Recebida> = estacao
        .colher(Duration::from_millis(700))
        .into_iter()
        .filter(|r| r.cab.tipo != Tipo::Medida)
        .collect();
    assert!(quadros.len() >= 3);
    assert!(
        quadros
            .iter()
            .all(|r| r.cab.tipo == Tipo::Entrada && r.carga.len() == 640 * 360 * 3)
    );
    let e = p.estado.lock().unwrap().clone();
    assert_eq!(e.format.as_deref(), Some("rgb"));
    assert!(
        e.encoder_problem
            .as_deref()
            .is_some_and(|m| m.contains("rms1-venc")),
        "{:?}",
        e.encoder_problem
    );

    // E o S1 sem codificador no arquivo, com a estação pedindo H.264: cru, e o motivo é outro.
    let porta = porta_livre();
    let estacao = Estacao::subir(
        porta,
        r#"[{"n":1,"id":"quadro_continuo","formato":"rgb_640x360","prefere":"h264","qualidade":0,"ritmo":10}]"#,
        SO_O_DETECTOR,
    );
    let p = subir_percepcao(porta, 22.0, Duration::from_secs(3));
    let quadros = estacao.colher(Duration::from_millis(500));
    assert!(quadros.iter().any(|r| r.cab.tipo == Tipo::Entrada));
    assert!(
        p.estado
            .lock()
            .unwrap()
            .encoder_problem
            .as_deref()
            .is_some_and(|m| m.contains("[midia] venc"))
    );
}

#[test]
fn a_estacao_que_recusa_faz_o_fluxo_recomecar_no_maximo_uma_vez_por_segundo() {
    let porta = porta_livre();
    let venc = VencFalso::subir();
    let estacao = Estacao::subir(
        porta,
        r#"[{"n":1,"id":"quadro_continuo","formato":"rgb_640x360","prefere":"h264","qualidade":4,"ritmo":20}]"#,
        SO_O_DETECTOR,
    );
    let _p = subir_percepcao_com(
        porta,
        22.0,
        Duration::from_secs(3),
        Some(venc.caminho.clone()),
    );
    estacao.colher(Duration::from_millis(400));
    let fluxos = |v: &VencFalso| v.pedidos().iter().filter(|x| x.0 == 3).count();
    assert_eq!(fluxos(&venc), 1, "um fluxo só enquanto a estação aceita");
    assert_eq!(
        venc.pedidos()[0],
        (3, 4 | (20 << 16), 0),
        "a taxa e o ritmo do anúncio"
    );
    // A estação recusa em todo quadro, por 2,3 s: o fluxo recomeça, mas não a cada quadro.
    let fim = Instant::now() + Duration::from_millis(2300);
    while Instant::now() < fim {
        estacao.recusar(1, "o quadro em h264 não abriu");
        std::thread::sleep(Duration::from_millis(50));
    }
    estacao.colher(Duration::from_millis(100));
    let n = fluxos(&venc);
    assert!(
        (2..=4).contains(&n),
        "{n} fluxos em 2,3 s de recusas: recomeça, e no máximo um por segundo"
    );
}

// ---- a ida e volta casada pelo número do quadro (2026-11-16) ---------------------------------------------------------------

const SO_UM_VIGIA: &str = r#"[{"n":1,"id":"vigia","entrega":"mascara"}]"#;
const QUADRO_A_20: &str = r#"[{"n":1,"id":"quadro_continuo","formato":"rgb_640x360","ritmo":20}]"#;

/// A estação responde a cada quadro `atraso` depois de ele chegar, por `por`: `com_entrega` diz se o quadro de número dado tem
/// entrega, e todos são marcados atendidos. Devolve quantas vezes o S1 pediu o atendido, e quantos quadros foram respondidos.
fn responder_atrasado(
    estacao: &Estacao,
    atraso: Duration,
    por: Duration,
    com_entrega: impl Fn(u16) -> bool,
) -> (usize, u16) {
    let fim = Instant::now() + por;
    let mut fila = std::collections::VecDeque::new();
    let (mut pedidos, mut n, mut respondidos) = (0usize, 0u16, 0u16);
    while Instant::now() < fim {
        while let Ok(r) = estacao.recebidas.try_recv() {
            match r.cab.tipo {
                Tipo::Atendido => pedidos += 1,
                Tipo::Entrada => {
                    fila.push_back((n, r.quando + atraso));
                    n += 1;
                }
                _ => {}
            }
        }
        while fila
            .front()
            .is_some_and(|(_, quando)| *quando <= Instant::now())
        {
            let (i, _) = fila.pop_front().unwrap();
            if com_entrega(i) {
                estacao.entregar(1, b"entrega");
            }
            estacao.atender(i);
            respondidos += 1;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    (pedidos, respondidos)
}

#[test]
fn com_o_quadro_marcado_atendido_a_ida_e_volta_e_a_do_quadro_e_nao_a_do_ultimo_envio() {
    // A 20 quadros por segundo sai um quadro a cada 50 ms, e a estação responde 150 ms depois de cada um: a resposta de um
    // quadro chega depois de mais dois ou três envios. Casada com o último envio, a medida daria menos de 50 ms.
    let porta = porta_livre();
    let estacao = Estacao::subir_com(porta, QUADRO_A_20, SO_UM_VIGIA, r#","atendido":true"#);
    let p = subir_percepcao(porta, 22.0, Duration::from_secs(3));
    let (pedidos, respondidos) = responder_atrasado(
        &estacao,
        Duration::from_millis(150),
        Duration::from_millis(1800),
        |_| true,
    );
    assert_eq!(pedidos, 1, "o S1 pede o atendido uma vez, ao ligar");
    assert!(
        respondidos >= 15,
        "{respondidos} quadros respondidos em 1,8 s"
    );
    let e = p.estado.lock().unwrap().clone();
    let ida = e.round_trip_ms.expect("a ida e volta medida");
    assert!(
        (120.0..260.0).contains(&ida),
        "ida e volta de {ida:.1} ms, com a resposta 150 ms depois de cada quadro"
    );
    assert_eq!(e.errors, 0, "marcar o quadro atendido não é erro");
}

#[test]
fn o_quadro_sem_entrega_nenhuma_nao_desloca_a_medida_dos_seguintes() {
    // Metade dos quadros não tem entrega (a função não rodou neles): são só marcados atendidos. A medida dos outros segue a
    // do próprio quadro.
    let porta = porta_livre();
    let estacao = Estacao::subir_com(porta, QUADRO_A_20, SO_UM_VIGIA, r#","atendido":true"#);
    let p = subir_percepcao(porta, 22.0, Duration::from_secs(3));
    let (_, respondidos) = responder_atrasado(
        &estacao,
        Duration::from_millis(150),
        Duration::from_millis(1800),
        |i| i % 2 == 0,
    );
    let e = p.estado.lock().unwrap().clone();
    let entregas = e.deliveries.get("vigia").copied().unwrap_or(0);
    assert!(
        entregas >= 7 && entregas <= u64::from(respondidos / 2 + 1),
        "{entregas} entregas em {respondidos} quadros"
    );
    let ida = e.round_trip_ms.expect("a ida e volta medida");
    assert!(
        (120.0..260.0).contains(&ida),
        "ida e volta de {ida:.1} ms, com entrega em um quadro sim e outro não"
    );
}

#[test]
fn a_estacao_que_nao_diz_que_marca_o_quadro_nao_recebe_o_pedido() {
    // Uma estação de antes disto fecharia a conexão ao receber um tipo de mensagem que não conhece.
    let porta = porta_livre();
    let estacao = Estacao::subir(porta, QUADRO_A_20, SO_UM_VIGIA);
    let _p = subir_percepcao(porta, 22.0, Duration::from_secs(3));
    let chegadas = estacao.colher(Duration::from_millis(600));
    assert!(
        chegadas
            .iter()
            .filter(|r| r.cab.tipo == Tipo::Entrada)
            .count()
            >= 4
    );
    assert!(
        chegadas.iter().all(|r| r.cab.tipo != Tipo::Atendido),
        "o S1 só pede o atendido a quem o anuncia"
    );
}
