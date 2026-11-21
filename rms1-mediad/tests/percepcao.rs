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
        self.mandar(Tipo::Entrega, canal, carga);
    }

    /// A estação manda uma mensagem ao S1.
    fn mandar(&self, tipo: Tipo, canal: u16, carga: &[u8]) {
        let mut c = self.conexao.lock().unwrap();
        let c = c.as_mut().expect("o S1 ligado");
        let cab = Cabecalho {
            tipo,
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
    subir_percepcao_inteira(
        porta_anuncio,
        ritmo_max,
        anuncio_vale,
        Pecas {
            venc,
            ..Pecas::default()
        },
    )
}

/// O que a percepção de um teste tem além da câmera: o codificador, a conversa (de onde vêm os trechos de voz, e para onde
/// vai o que a estação responde), quem toca a fala da estação, quem cumpre os pedidos dela, e a camada de mundo.
#[derive(Default)]
struct Pecas {
    venc: Option<std::path::PathBuf>,
    conversa: Option<percepcao::Conversa>,
    fala: Option<Arc<dyn Fn(percepcao::Som) + Send + Sync>>,
    agente: Option<percepcao::Agente>,
    mundo: Option<percepcao::Mundo>,
}

/// O mesmo, com as peças dadas.
fn subir_percepcao_inteira(
    porta_anuncio: u16,
    ritmo_max: f64,
    anuncio_vale: Duration,
    pecas: Pecas,
) -> Percepcao {
    let Pecas {
        venc,
        conversa,
        fala,
        agente,
        mundo,
    } = pecas;
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
            conversa,
            fala,
            agente,
            mundo,
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

#[test]
fn com_a_estacao_atras_o_s1_espera_a_marca_em_vez_de_empilhar_quadros() {
    // A estação atende um quadro de cada vez e leva 150 ms em cada um: dá conta de menos de 7 por segundo, e o anúncio pede 20.
    // Mandando no ritmo pedido, os quadros se empilham na rede e a detecção chega cada vez mais velha (no robô, 0,67 s de
    // atraso, medidos em 2026-11-16 com o ritmo em 22 e o yolov11s). O S1 só manda outro quadro com menos de dois sem marca.
    let porta = porta_livre();
    let estacao = Estacao::subir_com(porta, QUADRO_A_20, SO_UM_VIGIA, r#","atendido":true"#);
    let p = subir_percepcao(porta, 22.0, Duration::from_secs(3));
    let servico = Duration::from_millis(150);
    let fim = Instant::now() + Duration::from_millis(2500);
    let mut fila = std::collections::VecDeque::new();
    let (mut n, mut livre_em, mut maior_fila, mut atendidos) = (0u16, Instant::now(), 0usize, 0u32);
    while Instant::now() < fim {
        while let Ok(r) = estacao.recebidas.try_recv() {
            if r.cab.tipo == Tipo::Entrada {
                fila.push_back((n, r.quando));
                n += 1;
            }
        }
        maior_fila = maior_fila.max(fila.len());
        if let Some(&(i, chegou)) = fila.front() {
            // Um de cada vez: o serviço de um quadro começa quando ele chega e a estação está livre.
            let pronto = chegou.max(livre_em) + servico;
            if pronto <= Instant::now() {
                fila.pop_front();
                livre_em = pronto;
                estacao.entregar(1, b"entrega");
                estacao.atender(i);
                atendidos += 1;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let e = p.estado.lock().unwrap().clone();
    assert!(atendidos >= 10, "{atendidos} quadros atendidos em 2,5 s");
    assert!(
        maior_fila <= 2,
        "{maior_fila} quadros sem resposta ao mesmo tempo: o S1 empilhou"
    );
    let ida = e.round_trip_ms.expect("a ida e volta medida");
    assert!(
        ida < 450.0,
        "ida e volta de {ida:.0} ms com a estação levando 150 ms por quadro"
    );
    assert!(
        e.frames_held > 0,
        "o estado diz quantos envios esperaram pela estação"
    );
    assert_eq!(e.errors, 0);
}

// ---- a voz que vai à estação e o resultado que volta (2026-11-17) ----------------------------------------------------------

const SO_A_VOZ: &str = r#"[{"n":3,"id":"audio_voz","formato":"pcm16_16k"}]"#;
const QUADRO_E_VOZ: &str = r#"[{"n":1,"id":"quadro_continuo","formato":"rgb_640x360","ritmo":10},{"n":3,"id":"audio_voz","formato":"pcm16_16k"}]"#;
const SO_PENSAR: &str = r#"[{"n":4,"id":"pensar","entrega":"pedidos_validos"}]"#;

type Publicadas = Arc<Mutex<Vec<String>>>;

/// A percepção com a conversa: por onde o teste diz um trecho de voz, e o que foi publicado a quem assina a conversa.
fn subir_com_a_conversa(porta: u16) -> (Percepcao, mpsc::Sender<Vec<i16>>, Publicadas) {
    let (dizer, trechos) = mpsc::channel();
    let resultados = Arc::new(Mutex::new(Vec::new()));
    let r = resultados.clone();
    let conversa = percepcao::Conversa {
        trechos,
        publicar: Arc::new(move |linha| r.lock().unwrap().push(linha)),
    };
    let p = subir_percepcao_inteira(
        porta,
        22.0,
        Duration::from_secs(3),
        Pecas {
            conversa: Some(conversa),
            ..Pecas::default()
        },
    );
    (p, dizer, resultados)
}

#[test]
fn a_voz_vai_a_estacao_mesmo_sem_quadro_pedido_e_o_resultado_volta_a_quem_assina() {
    let porta = porta_livre();
    let estacao = Estacao::subir_com(
        porta,
        SO_A_VOZ,
        SO_PENSAR,
        r#","atendido":true,"sabe":true"#,
    );
    let (p, dizer, resultados) = subir_com_a_conversa(porta);
    // Ao ligar, o S1 pede a marca dos quadros e diz o que sabe receber; e não manda quadro, que a estação não pediu.
    let ao_ligar = estacao.colher(Duration::from_millis(700));
    assert_eq!(
        ao_ligar.iter().map(|r| r.cab.tipo).collect::<Vec<_>>(),
        [Tipo::Atendido, Tipo::Sabe],
        "o que chegou ao ligar"
    );
    let sabe: serde_json::Value = serde_json::from_slice(&ao_ligar[1].carga).unwrap();
    assert_eq!(sabe, serde_json::json!({"recebe": ["resultado"]}));
    // O trecho de voz vai como a entrada de voz do anúncio, em PCM de 16 bits.
    dizer.send(vec![1, -2, 300]).unwrap();
    let voz = estacao
        .recebidas
        .recv_timeout(Duration::from_secs(2))
        .expect("o trecho de voz");
    assert_eq!((voz.cab.tipo, voz.cab.canal), (Tipo::Entrada, 3));
    assert_eq!(voz.carga, [1, 0, 0xfe, 0xff, 0x2c, 0x01]);
    let e = p.estado.lock().unwrap().clone();
    assert_eq!(
        (e.idle_reason, e.rate, e.voice_sent, e.frames_sent),
        (None, None, 1, 0)
    );
    // O que a estação responde chega a quem assina a conversa, com a função e o que ela entrega; e não a quem vê as caixas.
    let fim = br#"{"pedido":7,"funcao":"pensar","o":"fim","resultado":"feito","entrega":{"ferramenta":"stop","parametros":{}}}"#;
    estacao.mandar(Tipo::Resultado, 4, fim);
    estacao.mandar(Tipo::Resultado, 9, fim);
    estacao.mandar(Tipo::Resultado, 4, b"nao e json");
    std::thread::sleep(Duration::from_millis(300));
    let linhas = resultados.lock().unwrap().clone();
    assert_eq!(linhas.len(), 1, "{linhas:?}");
    let v: serde_json::Value = serde_json::from_str(&linhas[0]).unwrap();
    assert_eq!(
        (v["jsonrpc"].as_str(), v["method"].as_str()),
        (Some("2.0"), Some("s1.station.result"))
    );
    assert!(v.get("id").is_none(), "é notificação, sem id");
    assert_eq!(
        (
            v["params"]["function"].as_str(),
            v["params"]["delivers"].as_str()
        ),
        (Some("pensar"), Some("pedidos_validos"))
    );
    assert_eq!(v["params"]["event"]["entrega"]["ferramenta"], "stop");
    assert!(p.publicadas.lock().unwrap().is_empty());
    let e = p.estado.lock().unwrap().clone();
    // O resultado de uma função que o anúncio não tem, e o que não é JSON, são contados como erro; a conexão segue.
    assert_eq!((e.results, e.errors), (1, 2));
    // Este S1 não disse que recebe a fala: se ela chegar, é erro de quem mandou, e nada toca.
    estacao.mandar(Tipo::Fala, 22_050, &[1, 0]);
    std::thread::sleep(Duration::from_millis(200));
    let e = p.estado.lock().unwrap().clone();
    assert_eq!((e.speech_received, e.errors), (0, 3));
    dizer.send(vec![7]).unwrap();
    assert!(
        estacao
            .recebidas
            .recv_timeout(Duration::from_secs(2))
            .is_ok(),
        "a conexão caiu"
    );
}

#[test]
fn a_estacao_que_nao_diz_que_sabe_nao_recebe_o_aviso_e_a_voz_vai_ao_lado_dos_quadros() {
    // Uma estação de antes fecharia a conexão ao receber um tipo de mensagem que não conhece.
    let porta = porta_livre();
    let estacao = Estacao::subir(porta, QUADRO_E_VOZ, SO_UM_VIGIA);
    let (p, dizer, _resultados) = subir_com_a_conversa(porta);
    // Tudo o que chega desde a ligação conta: o aviso, se fosse mandado, seria a primeira mensagem.
    std::thread::sleep(Duration::from_millis(700));
    dizer.send(vec![5; 1000]).unwrap();
    let chegadas = estacao.colher(Duration::from_millis(800));
    assert!(
        chegadas.iter().all(|r| r.cab.tipo != Tipo::Sabe),
        "o S1 só diz o que sabe a quem anuncia que sabe"
    );
    let vozes: Vec<&Recebida> = chegadas.iter().filter(|r| r.cab.canal == 3).collect();
    assert_eq!(vozes.len(), 1);
    assert_eq!(
        (vozes[0].cab.tipo, vozes[0].carga.len()),
        (Tipo::Entrada, 2000)
    );
    let quadros = chegadas
        .iter()
        .filter(|r| r.cab.tipo == Tipo::Entrada && r.cab.canal == 1)
        .count();
    assert!(quadros >= 5, "{quadros} quadros em 0,8 s, a 10 por segundo");
    let e = p.estado.lock().unwrap().clone();
    assert_eq!((e.voice_sent, e.rate, e.errors), (1, Some(10.0), 0));
}

#[test]
fn o_que_foi_dito_sem_estacao_nao_e_guardado_para_depois() {
    let porta = porta_livre();
    let (_p, dizer, _resultados) = subir_com_a_conversa(porta);
    std::thread::sleep(Duration::from_millis(200));
    dizer.send(vec![9; 100]).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    // A estação chega depois: o que foi dito antes já passou, e não vira pedido atrasado.
    let estacao = Estacao::subir_com(porta, SO_A_VOZ, SO_PENSAR, r#","sabe":true"#);
    let chegadas = estacao.colher(Duration::from_millis(900));
    assert!(
        chegadas.iter().any(|r| r.cab.tipo == Tipo::Sabe),
        "o S1 não ligou"
    );
    assert!(
        chegadas.iter().all(|r| r.cab.tipo != Tipo::Entrada),
        "a voz velha foi mandada"
    );
    // Sem microfone guardando voz, o S1 não liga a uma estação que só pede a voz.
    let porta = porta_livre();
    let estacao = Estacao::subir_com(porta, SO_A_VOZ, SO_PENSAR, r#","sabe":true"#);
    let p = subir_percepcao(porta, 22.0, Duration::from_secs(3));
    assert!(estacao.colher(Duration::from_millis(500)).is_empty());
    assert_eq!(
        p.estado.lock().unwrap().idle_reason.as_deref(),
        Some("a estação não pede nenhuma entrada que este S1 manda")
    );
}

// ---- a fala que vem da estação (2026-11-18) -------------------------------------------------------------------------------

type Sons = Arc<Mutex<Vec<percepcao::Som>>>;

/// A percepção de um S1 com alto-falante: o que a estação mandou tocar, na ordem. Com `com_microfone`, tem também a conversa.
fn subir_com_a_fala(porta: u16, com_microfone: bool) -> (Percepcao, Sons) {
    let sons: Sons = Arc::new(Mutex::new(Vec::new()));
    let s = sons.clone();
    let conversa = com_microfone.then(|| percepcao::Conversa {
        // Ninguém diz nada: quem diria os trechos já saiu.
        trechos: mpsc::channel::<Vec<i16>>().1,
        publicar: Arc::new(|_| {}),
    });
    let p = subir_percepcao_inteira(
        porta,
        22.0,
        Duration::from_secs(3),
        Pecas {
            conversa,
            fala: Some(Arc::new(move |som| s.lock().unwrap().push(som))),
            ..Pecas::default()
        },
    );
    (p, sons)
}

#[test]
fn a_fala_da_estacao_vai_a_quem_toca_na_taxa_e_na_ordem_em_que_chega() {
    use percepcao::Som;
    let porta = porta_livre();
    // A estação não pede nada que este S1 mande: ele liga assim mesmo, para ouvir o que ela fala.
    let estacao = Estacao::subir_com(porta, "[]", SO_PENSAR, r#","sabe":true"#);
    let (p, sons) = subir_com_a_fala(porta, false);
    let ao_ligar = estacao.colher(Duration::from_millis(700));
    assert_eq!(
        ao_ligar.iter().map(|r| r.cab.tipo).collect::<Vec<_>>(),
        [Tipo::Sabe],
        "o que chegou ao ligar"
    );
    let sabe: serde_json::Value = serde_json::from_slice(&ao_ligar[0].carga).unwrap();
    assert_eq!(sabe, serde_json::json!({"recebe": ["fala"]}));
    // O canal leva a taxa; a mensagem sem carga diz que a fala acabou (canal 0) ou foi interrompida (canal 1).
    estacao.mandar(Tipo::Fala, 22_050, &[1, 0, 2, 0]);
    estacao.mandar(Tipo::Fala, 22_050, &[3, 0]);
    estacao.mandar(Tipo::Fala, 0, &[]);
    estacao.mandar(Tipo::Fala, 16_000, &[4, 0]);
    estacao.mandar(Tipo::Fala, 1, &[]);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        *sons.lock().unwrap(),
        [
            Som::Pedaco {
                hz: 22_050,
                pcm: vec![1, 0, 2, 0]
            },
            Som::Pedaco {
                hz: 22_050,
                pcm: vec![3, 0]
            },
            Som::Fim {
                interrompida: false
            },
            Som::Pedaco {
                hz: 16_000,
                pcm: vec![4, 0]
            },
            Som::Fim { interrompida: true },
        ]
    );
    let e = p.estado.lock().unwrap().clone();
    assert_eq!(
        (e.speech_received, e.errors, e.idle_reason, e.frames_sent),
        (3, 0, None, 0)
    );
    // A conexão cai no meio de uma fala: quem toca fica sabendo que ela acabou, ou o microfone ficaria surdo à espera.
    estacao.mandar(Tipo::Fala, 22_050, &[5, 0]);
    std::thread::sleep(Duration::from_millis(200));
    estacao
        .conexao
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .shutdown(std::net::Shutdown::Both)
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        sons.lock().unwrap()[5..7],
        [
            Som::Pedaco {
                hz: 22_050,
                pcm: vec![5, 0]
            },
            Som::Fim {
                interrompida: false
            },
        ]
    );
}

#[test]
fn o_s1_com_microfone_e_alto_falante_diz_que_recebe_o_resultado_e_a_fala() {
    let porta = porta_livre();
    let estacao = Estacao::subir_com(porta, SO_A_VOZ, SO_PENSAR, r#","sabe":true"#);
    let (_p, _sons) = subir_com_a_fala(porta, true);
    let ao_ligar = estacao.colher(Duration::from_millis(700));
    let sabe: Vec<serde_json::Value> = ao_ligar
        .iter()
        .filter(|r| r.cab.tipo == Tipo::Sabe)
        .map(|r| serde_json::from_slice(&r.carga).unwrap())
        .collect();
    assert_eq!(sabe, [serde_json::json!({"recebe": ["resultado", "fala"]})]);
    // A estação de antes, que não diz que sabe, não recebe o aviso; e sem nada a mandar a ela, o S1 nem liga.
    let porta = porta_livre();
    let estacao = Estacao::subir(porta, "[]", SO_PENSAR);
    let (p, _sons) = subir_com_a_fala(porta, false);
    assert!(estacao.colher(Duration::from_millis(500)).is_empty());
    assert_eq!(
        p.estado.lock().unwrap().idle_reason.as_deref(),
        Some("a estação não pede nenhuma entrada que este S1 manda")
    );
}

// ---- o pedido que a estação manda cumprir, e o desfecho que volta (2026-11-18) ------------------------------------------

/// A percepção de um S1 com agente: os pedidos que chegaram a ele, e por onde o teste diz um desfecho. Com `ha_agente`
/// falso, ninguém assina os pedidos.
fn subir_com_o_agente(
    porta: u16,
    ha_agente: bool,
) -> (Percepcao, Publicadas, mpsc::Sender<String>) {
    let pedidos: Publicadas = Arc::new(Mutex::new(Vec::new()));
    let p = pedidos.clone();
    let (devolver, desfechos) = mpsc::channel();
    let agente = percepcao::Agente {
        pedir: Arc::new(move |linha| {
            p.lock().unwrap().push(linha);
            ha_agente
        }),
        desfechos,
        devolver: devolver.clone(),
    };
    let p = subir_percepcao_inteira(
        porta,
        22.0,
        Duration::from_secs(3),
        Pecas {
            agente: Some(agente),
            ..Pecas::default()
        },
    );
    (p, pedidos, devolver)
}

#[test]
fn o_pedido_da_estacao_vai_ao_agente_e_o_desfecho_volta_a_ela() {
    let porta = porta_livre();
    // A estação não pede nada que este S1 mande: ele liga assim mesmo, para cumprir o que ela pedir.
    let estacao = Estacao::subir_com(porta, "[]", SO_PENSAR, r#","sabe":true"#);
    let (p, pedidos, devolver) = subir_com_o_agente(porta, true);
    let ao_ligar = estacao.colher(Duration::from_millis(700));
    assert_eq!(
        ao_ligar.iter().map(|r| r.cab.tipo).collect::<Vec<_>>(),
        [Tipo::Sabe],
        "o que chegou ao ligar"
    );
    let sabe: serde_json::Value = serde_json::from_slice(&ao_ligar[0].carga).unwrap();
    assert_eq!(sabe, serde_json::json!({"recebe": ["pedido"]}));
    // O pedido chega a quem assina, com o número dele na estação, a ferramenta e os parâmetros.
    let pedido =
        br#"{"pedido":41,"ferramenta":"move","parametros":{"direction":"forward","meters":0.5}}"#;
    estacao.mandar(Tipo::Pedido, 0, pedido);
    estacao.mandar(Tipo::Pedido, 0, b"nao e json");
    estacao.mandar(Tipo::Pedido, 0, br#"{"ferramenta":"move"}"#);
    std::thread::sleep(Duration::from_millis(300));
    let linhas = pedidos.lock().unwrap().clone();
    assert_eq!(linhas.len(), 1, "{linhas:?}");
    let v: serde_json::Value = serde_json::from_str(&linhas[0]).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"jsonrpc": "2.0", "method": "s1.station.request", "params": {"request": 41, "tool": "move", "params": {"direction": "forward", "meters": 0.5}}})
    );
    // O desfecho que o agente diz volta à estação como chegou.
    let desfecho = r#"{"pedido":41,"ferramenta":"move","como":"cumprida","motivo":"andei 0.50 m"}"#;
    devolver.send(desfecho.to_owned()).unwrap();
    let volta = estacao
        .recebidas
        .recv_timeout(Duration::from_secs(2))
        .expect("o desfecho");
    assert_eq!(
        (volta.cab.tipo, volta.cab.canal, volta.carga.as_slice()),
        (Tipo::Desfecho, 0, desfecho.as_bytes())
    );
    // A estação manda largar o pedido (ela o interrompeu, ou o teto dela venceu): a ordem vai a quem assina, sem ferramenta.
    estacao.mandar(Tipo::Pedido, 0, br#"{"pedido":41,"cancelar":true}"#);
    std::thread::sleep(Duration::from_millis(300));
    let linhas = pedidos.lock().unwrap().clone();
    assert_eq!(linhas.len(), 2, "{linhas:?}");
    let v: serde_json::Value = serde_json::from_str(&linhas[1]).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"jsonrpc": "2.0", "method": "s1.station.request", "params": {"request": 41, "cancel": true}})
    );
    let e = p.estado.lock().unwrap().clone();
    // O pedido que não é JSON, e o que não diz o número, são contados como erro; a conexão segue.
    assert_eq!((e.requests, e.outcomes, e.errors), (2, 1, 2));

    // Sem agente assinando, o pedido é recusado na hora, dizendo por quê: a estação não fica à espera.
    let porta = porta_livre();
    let estacao = Estacao::subir_com(porta, "[]", SO_PENSAR, r#","sabe":true"#);
    let (_p, _pedidos, _devolver) = subir_com_o_agente(porta, false);
    assert_eq!(estacao.colher(Duration::from_millis(700)).len(), 1);
    estacao.mandar(Tipo::Pedido, 0, pedido);
    let volta = estacao
        .recebidas
        .recv_timeout(Duration::from_secs(2))
        .expect("a recusa");
    let d: serde_json::Value = serde_json::from_slice(&volta.carga).unwrap();
    assert_eq!(volta.cab.tipo, Tipo::Desfecho);
    assert_eq!(
        (
            d["pedido"].as_u64(),
            d["ferramenta"].as_str(),
            d["como"].as_str()
        ),
        (Some(41), Some("move"), Some("recusada"))
    );
    assert!(d["motivo"].as_str().unwrap().contains("agente"), "{d}");
    // A ordem de largar, sem agente, não tem o que largar: nada volta.
    estacao.mandar(Tipo::Pedido, 0, br#"{"pedido":41,"cancelar":true}"#);
    assert!(estacao.colher(Duration::from_millis(400)).is_empty());
}

// ---- a grade de profundidade que vira mundo (2026-11-20) ------------------------------------------------------------------

/// A cena de mentira do pacote do mundo: o chão, as caixas, e a grade que o `scdepthv3` daria delas. É a mesma dos testes de
/// lá, e não uma cópia.
#[path = "../../rms1-mundo/tests/comum/mod.rs"]
mod cena;

use rms1_mediad::corpo::Relogio;

const QUADRO_A_10: &str = r#"[{"n":1,"id":"quadro_continuo","formato":"rgb_640x360","ritmo":10}]"#;
/// Os Objetos e a Distância ligados, como a estação os anuncia: a Distância entrega a grade (`mapa`), pelo leitor do modelo.
const OBJETOS_E_DISTANCIA: &str = r#"[{"n":1,"id":"detector","entrega":"caixas","leitor":"caixas_padrao_hailo","publica":"media.detections","nota":0.35},{"n":2,"id":"profundidade","entrega":"mapa","leitor":"scdepthv3"}]"#;
/// A escala que o modelo esconde: nenhum número do S1 pode depender dela.
const K: f64 = 0.1;

/// A percepção com a camada de mundo: o mundo e o relógio dele, por onde o teste dá a torre e a pose (como o cliente do
/// `rms1d` as daria), e as linhas publicadas em `s1.world` e em `s1.world.grid`.
struct ComOMundo {
    p: Percepcao,
    mundo: Arc<Mutex<rms1_mundo::Mundo>>,
    relogio: Relogio,
    linhas: Publicadas,
    grades: Publicadas,
    /// A camada de mundo como a percepção a recebeu: é por ela que a grade de fora da estação entra.
    da_percepcao: percepcao::Mundo,
}

fn subir_com_o_mundo(porta: u16, ajustes: rms1_mundo::Ajustes) -> ComOMundo {
    subir_com_o_mundo_de(porta, ajustes, true)
}

/// `da_estacao`: se a grade que vira mundo é a que a estação entrega. Falso é o simulador com a profundidade da cena.
fn subir_com_o_mundo_de(porta: u16, ajustes: rms1_mundo::Ajustes, da_estacao: bool) -> ComOMundo {
    let mundo = Arc::new(Mutex::new(
        rms1_mundo::Mundo::novo(Some(cena::olho_do_simulador()), cena::corpo(), ajustes).unwrap(),
    ));
    let relogio = Relogio::default();
    let linhas: Publicadas = Arc::new(Mutex::new(Vec::new()));
    let grades: Publicadas = Arc::new(Mutex::new(Vec::new()));
    let (l, g) = (linhas.clone(), grades.clone());
    let da_percepcao = percepcao::Mundo {
        mundo: mundo.clone(),
        relogio,
        publicar: Arc::new(move |linha| l.lock().unwrap().push(linha)),
        publicar_grade: Arc::new(move |linha| g.lock().unwrap().push(linha)),
        quer: Arc::new(|| [true, true]),
        da_estacao,
    };
    let p = subir_percepcao_inteira(
        porta,
        22.0,
        Duration::from_secs(3),
        Pecas {
            mundo: Some(da_percepcao.clone()),
            ..Pecas::default()
        },
    );
    ComOMundo {
        p,
        mundo,
        relogio,
        linhas,
        grades,
        da_percepcao,
    }
}

impl ComOMundo {
    /// Dá ao mundo a torre e a pose, 100 vezes por segundo, até o teste acabar: `giro` diz o giro da torre, em graus, na
    /// hora dada. O corpo fica parado na origem, com a torre nivelada.
    fn dar_amostras(&self, giro: impl Fn(f64) -> f64 + Send + 'static) {
        let (m, relogio, parar) = (self.mundo.clone(), self.relogio, self.p.parar.clone());
        std::thread::spawn(move || {
            while !parar.load(Ordering::Relaxed) {
                let t = relogio.agora();
                m.lock()
                    .unwrap()
                    .amostra(cena::amostra(t, giro(t), 0.0, [0.0; 3]));
                std::thread::sleep(Duration::from_millis(10));
            }
        });
    }

    /// Os parâmetros da linha `s1.world` mais nova.
    fn ultima(&self) -> Option<serde_json::Value> {
        let linha = self.linhas.lock().unwrap().last().cloned()?;
        let v: serde_json::Value = serde_json::from_str(&linha).unwrap();
        assert_eq!(
            (v["jsonrpc"].as_str(), v["method"].as_str(), v.get("id")),
            (Some("2.0"), Some("s1.world"), None),
            "é notificação, sem id: {v}"
        );
        Some(v["params"].clone())
    }

    /// Espera o estado da percepção, em `s1.world`, ser `nome`; devolve os parâmetros da linha que o disse.
    fn esperar_estado(&self, nome: &str, por: Duration) -> serde_json::Value {
        let fim = Instant::now() + por;
        loop {
            let ultima = self.ultima();
            if let Some(u) = &ultima
                && u["perception"]["state"] == nome
            {
                return u.clone();
            }
            assert!(
                Instant::now() < fim,
                "o estado da percepção não chegou a {nome} em {por:?}: {ultima:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// O próximo quadro tirado de agora em diante, e o número dele. Os que esperavam na fila da estação são marcados atendidos,
/// sem nada: são de antes, e a grade de um quadro velho não acharia a torre do instante dele.
fn quadro_novo(estacao: &Estacao, n: &mut u16) -> (u16, Recebida) {
    while let Ok(r) = estacao.recebidas.try_recv() {
        if r.cab.tipo == Tipo::Entrada {
            estacao.atender(*n);
            *n += 1;
        }
    }
    loop {
        let r = estacao
            .recebidas
            .recv_timeout(Duration::from_secs(3))
            .expect("um quadro");
        if r.cab.tipo == Tipo::Entrada {
            *n += 1;
            return (*n - 1, r);
        }
    }
}

/// A marca de um quadro com o que aconteceu nele, como a estação a manda desde 2026-11-20.
fn marcar(estacao: &Estacao, n: u16, carga: &str) {
    estacao.mandar(Tipo::Atendido, n, carga.as_bytes());
}

/// A cena dos testes: uma caixa a 1 m, bem à frente do robô, e uma parede a 4 m, além do alcance do mapa.
fn caixa_a_um_metro() -> [cena::Caixa; 2] {
    [cena::caixa_em(1.0, 0.0), cena::parede_em(4.0)]
}

#[test]
fn a_grade_e_casada_com_a_torre_do_instante_em_que_o_quadro_foi_tirado_e_o_mundo_e_publicado() {
    let porta = porta_livre();
    let c = subir_com_o_mundo(porta, cena::ajustes());
    // A torre parada a 30 graus para a direita, até o primeiro quadro chegar à estação; daí em diante varre para a esquerda
    // a 60 graus por segundo.
    let comeco: Arc<Mutex<Option<f64>>> = Arc::default();
    let giro = {
        let comeco = comeco.clone();
        move |t: f64| {
            comeco
                .lock()
                .unwrap()
                .map_or(-30.0, |t0| (-30.0 + 60.0 * (t - t0)).min(30.0))
        }
    };
    c.dar_amostras(giro.clone());
    let estacao = Estacao::subir_com(
        porta,
        QUADRO_A_10,
        OBJETOS_E_DISTANCIA,
        r#","atendido":true"#,
    );
    let mut n = 0;
    let (primeiro, chegou) = quadro_novo(&estacao, &mut n);
    *comeco.lock().unwrap() = Some(c.relogio.de(chegou.quando));
    estacao.atender(primeiro);
    // O quadro seguinte é tirado com a torre em movimento. A estação vê a cena como a câmera a via naquele instante, leva
    // 300 ms para responder (18 graus de giro), e só então entrega a grade e marca o quadro.
    let (segundo, chegou) = quadro_novo(&estacao, &mut n);
    let t = c.relogio.de(chegou.quando);
    let na_captura = cena::amostra(t, giro(t), 0.0, [0.0; 3]);
    let grade = cena::ver(
        &cena::olho_do_simulador(),
        &na_captura,
        &caixa_a_um_metro(),
        K,
    );
    std::thread::sleep(Duration::from_millis(300));
    estacao.entregar(2, &grade);
    marcar(
        &estacao,
        segundo,
        r#"{"entregaram":[2],"falharam":[],"recusa":null}"#,
    );

    // A caixa está parada a 1 m, bem à frente do corpo (rumo zero). Casada com a torre do instante da captura, é lá que ela
    // cai no mapa; casada com a de quando a grade chegou, cairia 18 graus à esquerda.
    let agora = c.esperar_estado("ok", Duration::from_secs(3));
    let fim = Instant::now() + Duration::from_secs(2);
    let ocupadas = loop {
        let ultima = c.grades.lock().unwrap().last().cloned();
        let v: serde_json::Value = ultima.as_deref().map_or(serde_json::Value::Null, |l| {
            serde_json::from_str(l).unwrap()
        });
        let lado = v["params"]["cell_m"].as_f64().unwrap_or(0.0);
        let ocupadas: Vec<[f64; 2]> = v["params"]["cells"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|cel| cel[2] == 2)
            .map(|cel| {
                [
                    (cel[0].as_f64().unwrap() + 0.5) * lado,
                    (cel[1].as_f64().unwrap() + 0.5) * lado,
                ]
            })
            .collect();
        if !ocupadas.is_empty() {
            assert_eq!(
                (
                    v["method"].as_str(),
                    v["params"]["frame"].as_str(),
                    v.get("id")
                ),
                (Some("s1.world.grid"), Some("odom"), None),
                "{v}"
            );
            assert_eq!(v["params"]["pose"][0].as_f64(), Some(0.0), "{v}");
            break ocupadas;
        }
        assert!(
            Instant::now() < fim,
            "nenhuma célula ocupada foi publicada: {v}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    let meio = ocupadas.iter().fold([0.0, 0.0], |s, p| {
        [
            s[0] + p[0] / ocupadas.len() as f64,
            s[1] + p[1] / ocupadas.len() as f64,
        ]
    });
    let rumo = meio[1].atan2(meio[0]).to_degrees();
    assert!(
        rumo.abs() < 6.0,
        "o rumo da caixa saiu a {rumo:.1} graus, com ela a 0: a grade foi casada com a torre de outro instante (as ocupadas, \
         em volta de {meio:?})"
    );
    assert!(
        (meio[0] - 1.0).abs() < 0.08,
        "a face da caixa está a 1 m, e as ocupadas ficaram em volta de {meio:?}"
    );

    // `s1.world`: o mundo no corpo de agora. O corredor à frente está barrado pela caixa: da frente do chassi (a 0,16 m do
    // centro) até a face dela, a 1 m.
    assert_eq!(agora["frame"], "body", "{agora}");
    assert_eq!(agora["perception"]["scale"], "measured", "{agora}");
    assert!(agora["perception"]["reason"].is_null(), "{agora}");
    assert_eq!(agora["clearance"].as_array().map(Vec::len), Some(36));
    let a_frente = agora["obstacle"]["ahead_m"]
        .as_f64()
        .expect("o obstáculo à frente");
    assert!(
        (a_frente - 0.84).abs() < 0.06,
        "a folga à frente deu {a_frente:.2} m, com a caixa a 0,84 m da frente do chassi"
    );
    let campo = agora["lens"]["hfov_deg"].as_f64().unwrap();
    assert!(
        (campo - 120.0).abs() < 0.01,
        "a lente do simulador: {agora}"
    );
    assert!(agora["frame_age_ms"].as_f64().is_some_and(|i| i >= 300.0));

    // A entrega foi contada, sem erro, e o estado diz quanto a grade levou para virar mundo. A grade não é detecção: quem
    // assina só as detecções não recebe nada de novo.
    let e = c.p.estado.lock().unwrap().clone();
    assert_eq!((e.deliveries.get("profundidade"), e.errors), (Some(&1), 0));
    assert!(e.world_ms.is_some_and(|ms| ms > 0.0), "{:?}", e.world_ms);
    assert!(c.p.publicadas.lock().unwrap().is_empty());

    // As duas linhas saem no ritmo de `[midia.mundo]`: 20 e 5 por segundo.
    let (antes, antes_da_grade) = (
        c.linhas.lock().unwrap().len(),
        c.grades.lock().unwrap().len(),
    );
    std::thread::sleep(Duration::from_secs(1));
    let (por_s, da_grade_por_s) = (
        c.linhas.lock().unwrap().len() - antes,
        c.grades.lock().unwrap().len() - antes_da_grade,
    );
    assert!(
        (16..=22).contains(&por_s) && (3..=7).contains(&da_grade_por_s),
        "{por_s} linhas de s1.world e {da_grade_por_s} de s1.world.grid num segundo"
    );
}

#[test]
fn sem_a_camada_de_mundo_a_grade_e_so_contada_e_a_grade_que_nao_se_le_e_erro() {
    // O S1 sem a camada de mundo (a câmera de teste): a grade é contada, como até aqui, e nada é publicado.
    let porta = porta_livre();
    let p = subir_percepcao(porta, 22.0, Duration::from_secs(3));
    let estacao = Estacao::subir_com(
        porta,
        QUADRO_A_10,
        OBJETOS_E_DISTANCIA,
        r#","atendido":true"#,
    );
    let mut n = 0;
    let (quadro, _) = quadro_novo(&estacao, &mut n);
    let parada = cena::amostra(0.0, 0.0, 0.0, [0.0; 3]);
    let grade = cena::ver(&cena::olho_do_simulador(), &parada, &caixa_a_um_metro(), K);
    estacao.entregar(2, &grade);
    // A marca que diz a falha da grade, a quem não tem mundo, não é nada.
    marcar(
        &estacao,
        quadro,
        r#"{"entregaram":[2],"falharam":[{"n":2,"motivo":"nada"}],"recusa":null}"#,
    );
    std::thread::sleep(Duration::from_millis(300));
    let e = p.estado.lock().unwrap().clone();
    assert_eq!(
        (e.deliveries.get("profundidade"), e.errors, e.world_ms),
        (Some(&1), 0, None)
    );
    assert!(p.publicadas.lock().unwrap().is_empty());

    // Com a camada de mundo: a carga que não é uma grade conta a entrega e um erro, e o mundo fica como estava.
    let porta = porta_livre();
    let c = subir_com_o_mundo(porta, cena::ajustes());
    c.dar_amostras(|_| 0.0);
    let estacao = Estacao::subir_com(
        porta,
        QUADRO_A_10,
        OBJETOS_E_DISTANCIA,
        r#","atendido":true"#,
    );
    let mut n = 0;
    let (quadro, _) = quadro_novo(&estacao, &mut n);
    estacao.entregar(2, &grade[..grade.len() - 1]);
    estacao.atender(quadro);
    std::thread::sleep(Duration::from_millis(300));
    let e = c.p.estado.lock().unwrap().clone();
    assert_eq!(
        (e.deliveries.get("profundidade"), e.errors, e.world_ms),
        (Some(&1), 1, None)
    );
    // Sem grade nenhuma lida, o que o mundo diz é que não tem medida recente; e nenhuma célula.
    let agora = c.esperar_estado("stale", Duration::from_secs(1));
    assert!(agora["obstacle"].is_null(), "{agora}");
    assert!(
        c.mundo
            .lock()
            .unwrap()
            .celulas(c.relogio.agora())
            .is_empty()
    );
    // A mesma grade, inteira, num quadro novo: agora vira mundo.
    let (quadro, _) = quadro_novo(&estacao, &mut n);
    estacao.entregar(2, &grade);
    estacao.atender(quadro);
    c.esperar_estado("ok", Duration::from_secs(2));
}

/// No simulador com a profundidade da cena, a grade que vira mundo é a que chega de fora (a página a tira da cena e a manda
/// pelo socket), e a da estação é só contada: o modelo de profundidade não lê a imagem desenhada como lê a de uma câmera.
#[test]
fn no_simulador_a_grade_da_cena_vira_mundo_e_a_da_estacao_e_so_contada() {
    let porta = porta_livre();
    let c = subir_com_o_mundo_de(porta, cena::ajustes(), false);
    c.dar_amostras(|_| 0.0);
    let estacao = Estacao::subir_com(
        porta,
        QUADRO_A_10,
        OBJETOS_E_DISTANCIA,
        r#","atendido":true"#,
    );
    let parada = cena::amostra(0.0, 0.0, 0.0, [0.0; 3]);
    let grade = cena::ver(&cena::olho_do_simulador(), &parada, &caixa_a_um_metro(), K);

    // A grade que a estação entrega é contada, e não vira mundo: a percepção segue sem medida.
    let mut n = 0;
    let (quadro, _) = quadro_novo(&estacao, &mut n);
    estacao.entregar(2, &grade);
    estacao.atender(quadro);
    std::thread::sleep(Duration::from_millis(400));
    let e = c.p.estado.lock().unwrap().clone();
    assert_eq!(
        (e.deliveries.get("profundidade"), e.world_ms, e.errors),
        (Some(&1), None, 0),
        "a grade da estação tem de ser só contada"
    );
    assert_eq!(
        c.ultima().unwrap()["perception"]["state"],
        "stale",
        "com a profundidade da cena, a grade da estação não vira mundo"
    );

    // A grade da cena, de fora: vira mundo, com a caixa a 0,84 m da frente do chassi, e é contada com o nome dela.
    let receber = percepcao::grade_de_fora(&c.da_percepcao, c.p.estado.clone());
    receber(&grade).unwrap();
    let agora = c.esperar_estado("ok", Duration::from_secs(2));
    assert_eq!(agora["perception"]["scale"], "measured", "{agora}");
    let a_frente = agora["obstacle"]["ahead_m"]
        .as_f64()
        .expect("o obstáculo à frente");
    assert!(
        (a_frente - 0.84).abs() < 0.06,
        "com a grade da cena, a folga à frente deu {a_frente:.2} m, com a caixa a 0,84 m da frente do chassi"
    );
    let e = c.p.estado.lock().unwrap().clone();
    assert_eq!(
        e.deliveries.get(percepcao::PROFUNDIDADE_DA_CENA),
        Some(&1),
        "{:?}",
        e.deliveries
    );
    assert!(e.world_ms.is_some_and(|ms| ms > 0.0), "{:?}", e.world_ms);

    // A que não se lê é recusada com o motivo, e conta erro.
    let motivo = receber(&grade[..100]).unwrap_err();
    assert!(motivo.contains("bytes"), "{motivo}");
    assert_eq!(c.p.estado.lock().unwrap().errors, 1);
}

/// Uma pasta de teste que some ao sair, com o que tiver dentro.
struct PastaDeTeste(std::path::PathBuf);

impl Drop for PastaDeTeste {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn o_estado_diz_a_hora_do_mundo_e_se_a_sessao_esta_sendo_gravada() {
    // Sem `gravar`: a hora é a do relógio das amostras, e a sessão não é gravada.
    let c = subir_com_o_mundo(porta_livre(), cena::ajustes());
    std::thread::sleep(Duration::from_millis(300));
    let e = c.p.estado.lock().unwrap().clone();
    let hora = e.world_clock_s.expect("a hora do mundo");
    let atraso = c.relogio.agora() - hora;
    assert!(
        (0.0..0.2).contains(&atraso),
        "a hora do mundo no estado está {atraso:.3} s atrás do relógio das amostras"
    );
    assert_eq!(e.world_recording, Some(false));
    // A hora anda.
    std::thread::sleep(Duration::from_millis(300));
    let depois = c.p.estado.lock().unwrap().world_clock_s.unwrap();
    assert!(
        depois - hora > 0.2,
        "a hora do mundo foi de {hora:.3} a {depois:.3} em 0,3 s"
    );

    // Com `gravar`: o estado diz que a sessão está sendo gravada.
    let pasta = PastaDeTeste(
        std::env::temp_dir().join(format!("rms1-mediad-gravando-{}", std::process::id())),
    );
    std::fs::create_dir_all(&pasta.0).unwrap();
    let ajustes = rms1_mundo::Ajustes {
        gravar: pasta.0.join("sessao.mundo").to_string_lossy().into_owned(),
        ..cena::ajustes()
    };
    let c = subir_com_o_mundo(porta_livre(), ajustes);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(c.p.estado.lock().unwrap().world_recording, Some(true));
}

#[test]
fn a_marca_que_diz_a_falha_da_grade_vira_failed_e_a_grade_que_para_de_chegar_vira_stale() {
    let porta = porta_livre();
    let c = subir_com_o_mundo(porta, cena::ajustes());
    c.dar_amostras(|_| 0.0);
    let estacao = Estacao::subir_com(
        porta,
        QUADRO_A_10,
        OBJETOS_E_DISTANCIA,
        r#","atendido":true"#,
    );
    let parada = cena::amostra(0.0, 0.0, 0.0, [0.0; 3]);
    let grade = cena::ver(&cena::olho_do_simulador(), &parada, &caixa_a_um_metro(), K);
    let mut n = 0;
    let (quadro, _) = quadro_novo(&estacao, &mut n);
    estacao.entregar(2, &grade);
    marcar(
        &estacao,
        quadro,
        r#"{"entregaram":[2],"falharam":[],"recusa":null}"#,
    );
    c.esperar_estado("ok", Duration::from_secs(3));

    // No quadro seguinte quem falhou foram os Objetos: a grade não falhou, e o mundo segue como estava.
    let (quadro, _) = quadro_novo(&estacao, &mut n);
    marcar(
        &estacao,
        quadro,
        r#"{"entregaram":[],"falharam":[{"n":1,"motivo":"o detector não respondeu"}],"recusa":null}"#,
    );
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(c.ultima().unwrap()["perception"]["state"], "ok");

    // Agora a Distância falha: a percepção falhou, com o motivo que a estação deu. Não é o mesmo que não ver nada.
    let (quadro, _) = quadro_novo(&estacao, &mut n);
    marcar(
        &estacao,
        quadro,
        r#"{"entregaram":[1],"falharam":[{"n":2,"motivo":"o modelo de profundidade não respondeu"}],"recusa":null}"#,
    );
    let agora = c.esperar_estado("failed", Duration::from_secs(1));
    assert_eq!(
        agora["perception"]["reason"],
        "o modelo de profundidade não respondeu"
    );
    assert!(
        agora["perception"]["scale"].is_null(),
        "sem medida, não há escala a dizer: {agora}"
    );

    // Uma grade boa depois da falha: a percepção voltou.
    let (quadro, _) = quadro_novo(&estacao, &mut n);
    estacao.entregar(2, &grade);
    marcar(
        &estacao,
        quadro,
        r#"{"entregaram":[2],"falharam":[],"recusa":null}"#,
    );
    c.esperar_estado("ok", Duration::from_secs(2));

    // O quadro recusado inteiro também é falha da percepção.
    let (quadro, _) = quadro_novo(&estacao, &mut n);
    marcar(
        &estacao,
        quadro,
        r#"{"entregaram":[],"falharam":[],"recusa":"o quadro em jpeg não abriu"}"#,
    );
    let agora = c.esperar_estado("failed", Duration::from_secs(1));
    assert_eq!(agora["perception"]["reason"], "o quadro em jpeg não abriu");

    // A marca que não se lê é contada como erro, e não muda o que o mundo diz.
    let (quadro, _) = quadro_novo(&estacao, &mut n);
    marcar(&estacao, quadro, "nao e json");
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(c.p.estado.lock().unwrap().errors, 1);

    // E a grade para de chegar: passado `velho_s` (1,5 s), a percepção está velha. Nunca "livre": o que o mundo lembra segue
    // no mapa, com a idade, e o estado diz que não há medida nova.
    let agora = c.esperar_estado("stale", Duration::from_secs(4));
    assert!(agora["perception"]["reason"].is_null(), "{agora}");
    assert!(
        agora["obstacle"]["ahead_m"].as_f64().is_some(),
        "a caixa vista segue lembrada à frente: {agora}"
    );
}
