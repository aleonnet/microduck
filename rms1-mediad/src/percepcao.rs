//! A percepção pela estação Hailo: o S1 captura, a estação pensa, e o S1 publica o que ela achou.
//!
//! O S1 não lê o contrato das funções. Ele ouve o anúncio da estação na rede ([`crate::estacao`]), que diz que entradas ela
//! quer e em que ritmo, e o que cada função ligada entrega. Para cada entrada que este S1 sabe mandar, manda; para cada
//! entrega que sabe ler, lê e, se a função pede, publica.
//!
//! Nesta frente o S1 sabe mandar o quadro contínuo (`quadro_continuo`, RGB 640 por 360, sem moldura: `QUADRO_LARGURA` e `QUADRO_ALTURA`, abaixo) e ler as caixas do
//! detector de patos (`caixas`, do leitor `nanodet_v8`), que publica em `media.detections`, no formato do pato.
//!
//! **O ritmo é o da estação**, até o teto de `[midia] percepcao.ritmo_max`. Os 2 por segundo do detector do pato são calor
//! na placa dele (`mediad/src/detect.rs:11-13`); aqui o Hailo está no Pi.
//!
//! Sem estação, nada é mandado, e `s1.media.status` diz por quê. Conexão caída reconecta com espera crescente.
//!
//! **A conversa.** O S1 que guarda a voz do microfone ([`Conversa`]) manda cada trecho como a entrada `audio_voz` (PCM de
//! 16 bits, 16 kHz), mesmo que a estação não peça quadro nenhum. À estação que diz que sabe (`sabe`, no anúncio), ele diz
//! ao ligar o que recebe ([`Tipo::Sabe`]); e cada acontecimento do pedido que a voz começou lá ([`Tipo::Resultado`]) é
//! publicado como `s1.station.result`, com a função e o tipo do que ela entrega. O que foi dito sem estação não é guardado.
//!
//! **A fala.** O S1 com alto-falante ([`Config::fala`]) diz à estação que recebe a fala, e cada pedaço dela
//! ([`Tipo::Fala`]: PCM de 16 bits, um canal, com a taxa no canal da mensagem) vai a quem toca, na ordem em que chega; a
//! mensagem sem carga diz que a fala acabou (canal 0) ou foi interrompida (canal 1). Para ouvir a fala, o S1 liga à estação
//! mesmo que ela não peça nada que ele mande. É toda fala da estação, e não só a do pedido que este S1 começou.
//!
//! **O pedido a cumprir.** O S1 com quem cumpra pedidos ([`Config::agente`]) diz à estação que recebe o pedido; cada pedido
//! validado que ela manda ([`Tipo::Pedido`]) é publicado como `s1.station.request`, e o desfecho que o agente diz volta a
//! ela ([`Tipo::Desfecho`]). Sem ninguém assinando os pedidos, a recusa volta na hora: a estação não fica à espera.

use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use duck_detect::{Detection, Letterbox};

use crate::estacao::{self, Anuncio, Cabecalho, Tipo};
use crate::quadros::Quadros;
use crate::reducao;
use crate::venc::Venc;

/// O quadro que vai à estação: 640 por 360, o tamanho que o pipeline fixa para a percepção (e o que a DJI entrega à dela).
pub const QUADRO_LARGURA: usize = 640;
pub const QUADRO_ALTURA: usize = 360;
/// A moldura em que a estação devolve as caixas, seja qual for o lado do modelo: o quadro como 320 por 180 no meio de
/// 320 por 320.
pub const LARGURA: usize = 320;
pub const ALTURA: usize = 180;
const LADO: usize = 320;
const BORDA_Y: f32 = ((LADO - ALTURA) / 2) as f32;
/// A resposta do detector: dois inteiros e até 50 caixas de cinco números (`tools/hailo/hailo_srv.cpp`, no laboratório).
const CAIXAS_MAXIMO: usize = 50;
const CAIXAS_BYTES: usize = 8 + CAIXAS_MAXIMO * 5 * 4;
/// A entrega do leitor `caixas_padrao_hailo`: o mesmo, com a classe a mais em cada caixa, já suprimida no Hailo
/// (`tools/hailo/servico/leitor_caixas_padrao.cpp`, no laboratório).
const CAIXAS_COM_CLASSE_BYTES: usize = 8 + CAIXAS_MAXIMO * 6 * 4;
/// Os nomes das 80 classes do COCO, na ordem em que os detectores do catálogo da Hailo as numeram.
pub const CLASSES_COCO: [&str; 80] = [
    "person",
    "bicycle",
    "car",
    "motorcycle",
    "airplane",
    "bus",
    "train",
    "truck",
    "boat",
    "traffic light",
    "fire hydrant",
    "stop sign",
    "parking meter",
    "bench",
    "bird",
    "cat",
    "dog",
    "horse",
    "sheep",
    "cow",
    "elephant",
    "bear",
    "zebra",
    "giraffe",
    "backpack",
    "umbrella",
    "handbag",
    "tie",
    "suitcase",
    "frisbee",
    "skis",
    "snowboard",
    "sports ball",
    "kite",
    "baseball bat",
    "baseball glove",
    "skateboard",
    "surfboard",
    "tennis racket",
    "bottle",
    "wine glass",
    "cup",
    "fork",
    "knife",
    "spoon",
    "bowl",
    "banana",
    "apple",
    "sandwich",
    "orange",
    "broccoli",
    "carrot",
    "hot dog",
    "pizza",
    "donut",
    "cake",
    "chair",
    "couch",
    "potted plant",
    "bed",
    "dining table",
    "toilet",
    "tv",
    "laptop",
    "mouse",
    "remote",
    "keyboard",
    "cell phone",
    "microwave",
    "oven",
    "toaster",
    "sink",
    "refrigerator",
    "book",
    "clock",
    "vase",
    "scissors",
    "teddy bear",
    "hair drier",
    "toothbrush",
];
const ESPERA_MIN: Duration = Duration::from_millis(100);
const ESPERA_MAX: Duration = Duration::from_secs(2);
/// Quantos quadros uma conexão tem de levar para a espera de religar voltar ao mínimo.
const QUADROS_DE_PROVA: u32 = 30;

pub struct Config {
    /// A porta UDP do anúncio.
    pub porta_anuncio: u16,
    /// O teto de quadros por segundo, seja qual for o ritmo anunciado.
    pub ritmo_max: f64,
    /// Sem anúncio por este tempo, a estação saiu.
    pub anuncio_vale: Duration,
    /// O codificador de hardware do S1, se o robô o tem (`[midia] venc`). Sem ele, o quadro vai cru.
    pub venc: Option<Arc<Mutex<Venc>>>,
    /// A voz que vai à estação e o que ela responde, se este S1 guarda a voz do microfone.
    pub conversa: Option<Conversa>,
    /// Quem toca a fala que a estação manda, se este S1 tem alto-falante.
    pub fala: Option<Arc<dyn Fn(Som) + Send + Sync>>,
    /// Quem cumpre no corpo o pedido que a estação manda.
    pub agente: Option<Agente>,
}

/// O caminho até quem cumpre os pedidos da estação.
pub struct Agente {
    /// Recebe cada notificação `s1.station.request`, uma linha JSON, e diz se alguém a recebeu.
    pub pedir: Arc<dyn Fn(String) -> bool + Send + Sync>,
    /// De onde vêm os desfechos, cada um o JSON que a estação lê.
    pub desfechos: Receiver<String>,
    /// Por onde entra em `desfechos` a recusa do pedido que ninguém recebeu.
    pub devolver: std::sync::mpsc::Sender<String>,
}

/// O que a estação manda tocar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Som {
    /// Um pedaço da fala: PCM de 16 bits, um canal, a `hz` amostras por segundo.
    Pedaco { hz: u32, pcm: Vec<u8> },
    /// A fala acabou; ou foi interrompida, e para agora.
    Fim { interrompida: bool },
}

/// A conversa com a estação: de onde vêm os trechos de voz (PCM de 16 bits, 16 kHz, um canal), e quem recebe cada
/// notificação `s1.station.result`, uma linha JSON.
pub struct Conversa {
    pub trechos: Receiver<Vec<i16>>,
    pub publicar: Arc<dyn Fn(String) + Send + Sync>,
}

/// O que a percepção está fazendo: é o que `s1.media.status` mostra.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Estado {
    /// O endereço da estação, se há uma na rede.
    pub station: Option<String>,
    /// Por que nada está sendo mandado, se não está.
    pub idle_reason: Option<String>,
    /// O ritmo em uso, em quadros por segundo.
    pub rate: Option<f64>,
    pub frames_sent: u64,
    /// Entregas recebidas, por função.
    pub deliveries: BTreeMap<String, u64>,
    /// Mensagens de erro da estação, e mensagens que este S1 não soube ler.
    pub errors: u64,
    /// Do envio do quadro à primeira entrega dele, em milissegundos.
    pub round_trip_ms: Option<f64>,
    /// Em que formato o último quadro foi: `rgb`, `h264` ou `jpeg`.
    pub format: Option<String>,
    /// Por que o quadro foi cru, tendo a estação pedido outro formato.
    pub encoder_problem: Option<String>,
    /// Quantos envios esperaram a estação alcançar: ela estava com dois quadros ainda sem resposta.
    pub frames_held: u64,
    /// Trechos de voz mandados à estação.
    pub voice_sent: u64,
    /// Acontecimentos de pedidos que a estação mandou de volta.
    pub results: u64,
    /// Pedaços de fala que a estação mandou tocar.
    pub speech_received: u64,
    /// Pedidos que a estação mandou cumprir, e desfechos mandados de volta.
    pub requests: u64,
    pub outcomes: u64,
}

/// As candidatas da resposta do detector, passadas do formato da estação (cinco números por caixa) para o que
/// `duck_detect::decode` lê: todos os cx, depois todos os cy, e assim por diante.
pub fn candidatas_em_blocos(carga: &[u8]) -> Result<Vec<f32>, String> {
    if carga.len() != CAIXAS_BYTES {
        return Err(format!(
            "resposta do detector com {} bytes, e não {CAIXAS_BYTES}",
            carga.len()
        ));
    }
    let inteiro =
        |i: usize| i32::from_le_bytes([carga[i], carga[i + 1], carga[i + 2], carga[i + 3]]);
    let vao = inteiro(4);
    if !(0..=CAIXAS_MAXIMO as i32).contains(&vao) {
        return Err(format!(
            "resposta do detector diz {vao} caixas, e o máximo é {CAIXAS_MAXIMO}"
        ));
    }
    let n = vao as usize;
    let numero = |k: usize| {
        let i = 8 + k * 4;
        f32::from_le_bytes([carga[i], carga[i + 1], carga[i + 2], carga[i + 3]])
    };
    let mut blocos = vec![0f32; n * 5];
    for caixa in 0..n {
        for campo in 0..5 {
            blocos[campo * n + caixa] = numero(caixa * 5 + campo);
        }
    }
    Ok(blocos)
}

/// As detecções de uma resposta do detector, nos pixels do quadro que a câmera tirou (`largura` por `altura`).
pub fn deteccoes(
    carga: &[u8],
    largura: u32,
    altura: u32,
    nota: f32,
    iou: f32,
) -> Result<Vec<Detection>, String> {
    let blocos = candidatas_em_blocos(carga)?;
    // A supressão é a do pato, na moldura; a volta ao quadro é daqui, porque a redução e a moldura são duas contas.
    let na_moldura = duck_detect::decode(
        &blocos,
        Letterbox {
            scale: 1.0,
            pad_x: 0.0,
            pad_y: 0.0,
        },
        nota,
        iou,
    );
    let (ex, ey) = (
        LARGURA as f32 / largura as f32,
        ALTURA as f32 / altura as f32,
    );
    Ok(na_moldura
        .into_iter()
        .map(|d| Detection {
            score: d.score,
            box_: [
                d.box_[0] / ex,
                (d.box_[1] - BORDA_Y) / ey,
                d.box_[2] / ex,
                (d.box_[3] - BORDA_Y) / ey,
            ],
        })
        .collect())
}

/// As detecções de uma entrega do leitor `caixas_padrao_hailo`, cada uma com a classe, nos pixels do quadro que a câmera
/// tirou. A supressão já foi feita no Hailo, e as caixas vêm da maior nota para a menor: aqui só entram o filtro da nota
/// da função e a volta ao quadro, pela mesma conta de [`deteccoes`].
pub fn deteccoes_com_classe(
    carga: &[u8],
    largura: u32,
    altura: u32,
    nota: f32,
) -> Result<Vec<(Detection, u32)>, String> {
    if carga.len() != CAIXAS_COM_CLASSE_BYTES {
        return Err(format!(
            "resposta do detector com {} bytes, e não {CAIXAS_COM_CLASSE_BYTES}",
            carga.len()
        ));
    }
    let vao = i32::from_le_bytes([carga[4], carga[5], carga[6], carga[7]]);
    if !(0..=CAIXAS_MAXIMO as i32).contains(&vao) {
        return Err(format!(
            "resposta do detector diz {vao} caixas, e o máximo é {CAIXAS_MAXIMO}"
        ));
    }
    let numero = |k: usize| {
        let i = 8 + k * 4;
        f32::from_le_bytes([carga[i], carga[i + 1], carga[i + 2], carga[i + 3]])
    };
    let (ex, ey) = (
        LARGURA as f32 / largura as f32,
        ALTURA as f32 / altura as f32,
    );
    let mut achadas = Vec::new();
    for caixa in 0..vao as usize {
        let [cx, cy, w, h, n, classe] = std::array::from_fn(|campo| numero(caixa * 6 + campo));
        if n < nota || !(0.0..65536.0).contains(&classe) {
            continue;
        }
        achadas.push((
            Detection {
                score: n,
                box_: [
                    (cx - w / 2.0) / ex,
                    (cy - h / 2.0 - BORDA_Y) / ey,
                    (cx + w / 2.0) / ex,
                    (cy + h / 2.0 - BORDA_Y) / ey,
                ],
            },
            classe as u32,
        ));
    }
    Ok(achadas)
}

/// `media.detections` com a classe: o formato do pato, e em cada caixa o número e o nome da classe do COCO.
pub fn notificacao_com_classe(
    achadas: &[(Detection, u32)],
    largura: u32,
    altura: u32,
    levou_ms: f64,
) -> String {
    let caixas: Vec<String> = achadas
        .iter()
        .map(|(d, classe)| {
            let nome = CLASSES_COCO.get(*classe as usize).copied().unwrap_or("");
            format!(
                r#"{{"x0":{:.1},"y0":{:.1},"x1":{:.1},"y1":{:.1},"score":{:.3},"class":{classe},"label":"{nome}"}}"#,
                d.box_[0], d.box_[1], d.box_[2], d.box_[3], d.score
            )
        })
        .collect();
    format!(
        r#"{{"jsonrpc":"2.0","method":"media.detections","params":{{"width":{largura},"height":{altura},"took_ms":{levou_ms:.1},"boxes":[{}]}}}}"#,
        caixas.join(",")
    )
}

/// `media.detections`, no formato do pato (`mediad/src/detect.rs`, `notification`): as caixas nos pixels do quadro, e o
/// tamanho do quadro junto. Sem caixa, a notificação sai assim mesmo: é como quem assina sabe que o robô olhou e não viu.
pub fn notificacao(achadas: &[Detection], largura: u32, altura: u32, levou_ms: f64) -> String {
    let caixas: Vec<String> = achadas
        .iter()
        .map(|d| {
            format!(
                r#"{{"x0":{:.1},"y0":{:.1},"x1":{:.1},"y1":{:.1},"score":{:.3}}}"#,
                d.box_[0], d.box_[1], d.box_[2], d.box_[3], d.score
            )
        })
        .collect();
    format!(
        r#"{{"jsonrpc":"2.0","method":"media.detections","params":{{"width":{largura},"height":{altura},"took_ms":{levou_ms:.1},"boxes":[{}]}}}}"#,
        caixas.join(",")
    )
}

/// O que as duas pontas da conexão partilham.
#[derive(Default)]
struct Partilha {
    anuncio: Option<Anuncio>,
    /// Quando o último quadro saiu, e o tamanho do quadro que a câmera tirou.
    ultimo_envio: Option<(Instant, u32, u32)>,
    /// Se a ida e volta desse quadro já foi medida: só a primeira entrega dele mede.
    medido: bool,
    /// A estação recusou uma mensagem: se o quadro vai em H.264, o fluxo recomeça por um quadro inteiro.
    recusou: bool,
    /// Nesta conexão a estação marca cada quadro atendido ([`Tipo::Atendido`]): a ida e volta casa pelo número do quadro, e
    /// não pelo último envio, que erra quando a resposta de um quadro chega depois do envio do seguinte.
    marcando: bool,
    /// Os quadros mandados e ainda não marcados atendidos, do mais antigo ao mais novo: o número de cada um, contado desde o
    /// pedido, e quando saiu.
    em_voo: VecDeque<(u16, Instant)>,
    /// O número do próximo quadro a sair.
    proximo: u16,
    /// Se o quadro da frente da fila já teve a ida e volta medida: só a primeira entrega dele mede.
    frente_medida: bool,
}

/// Quantos quadros esperam a marca da estação, no máximo: acima disto ela parou de marcar, e os mais antigos saem da fila.
const EM_VOO_MAX: usize = 1024;
/// Com este tanto de quadros ainda sem a marca da estação, o S1 não manda outro: espera. A estação que não dá conta do ritmo
/// pedido receberia os quadros empilhados na rede, e a detecção chegaria cada vez mais velha (0,67 s de atraso no robô, em
/// 2026-11-16, com o ritmo em 22 e um modelo que dá 18 por segundo). Assim o ritmo passa a ser o que ela aguenta, e o quadro
/// que ela vê é sempre o mais novo: ele é tirado da câmera na hora de sair.
const EM_VOO_LIMITE: usize = 2;
/// Um quadro sem marca há mais que isto foi perdido pela estação: deixa de contar, para o envio não parar de vez.
const SEM_MARCA: Duration = Duration::from_secs(1);

/// Roda até `parar`. `publicar` recebe cada notificação pronta, uma linha JSON.
pub fn rodar(
    cfg: Config,
    quadros: Quadros,
    publicar: impl Fn(String) + Send + Sync + 'static,
    estado: Arc<Mutex<Estado>>,
    parar: Arc<AtomicBool>,
) -> std::io::Result<()> {
    let udp = ouvir(cfg.porta_anuncio)?;
    let publicar = Arc::new(publicar);
    let partilha = Arc::new(Mutex::new(Partilha::default()));
    let mut vista: Option<(SocketAddr, Instant)> = None;
    let mut conexao: Option<(TcpStream, SocketAddr)> = None;
    let mut espera = ESPERA_MIN;
    let mut na_conexao = 0u32;
    let mut tentar_em = Instant::now();
    let mut proximo = Instant::now();
    let mut medida_em = Instant::now();
    let mut no_segundo = 0u32;
    // O fluxo de H.264 em curso: a taxa, os quadros por segundo, e quando começou. `None`: o próximo quadro começa um.
    let mut fluxo: Option<(u32, u32, Instant)> = None;
    let mut ultimo_problema: Option<String> = None;
    // Se o envio da vez está esperando a estação alcançar: conta uma vez por envio que esperou.
    let mut segurando = false;
    let mut pacote = [0u8; 65535];
    let dizer = |por_que: Option<&str>, estacao: Option<IpAddr>, ritmo: Option<f64>| {
        let mut e = estado.lock().expect("estado da percepção");
        e.idle_reason = por_que.map(str::to_owned);
        e.station = estacao.map(|ip| ip.to_string());
        e.rate = ritmo;
    };

    while !parar.load(Ordering::Relaxed) {
        let agora = Instant::now();
        // O que foi dito desde a última volta vai nesta, ou não vai: sem estação ligada, a fala não vira pedido atrasado.
        let trechos: Vec<Vec<i16>> = cfg
            .conversa
            .as_ref()
            .map(|c| c.trechos.try_iter().collect())
            .unwrap_or_default();
        while let Ok((n, de)) = udp.recv_from(&mut pacote) {
            if let Some(a) = estacao::ler_anuncio(&pacote[..n], de.ip()) {
                // Com duas estações na rede, o S1 fica com a que já usa enquanto ela se anuncia: trocar a cada pacote
                // seria reconectar duas vezes por segundo. Uma estação é o endereço e a porta dos quadros.
                let esta = SocketAddr::new(de.ip(), a.quadros);
                if vista.is_none_or(|(em_uso, _)| em_uso == esta) {
                    partilha.lock().expect("partilha").anuncio = Some(a);
                    vista = Some((esta, agora));
                }
            }
        }
        if vista.is_some_and(|(_, quando)| agora.duration_since(quando) >= cfg.anuncio_vale) {
            vista = None;
            fechar(&mut conexao);
            partilha.lock().expect("partilha").anuncio = None;
        }
        let Some((alvo, _)) = vista else {
            dizer(Some("nenhuma estação se anunciou na rede"), None, None);
            std::thread::sleep(Duration::from_millis(20));
            continue;
        };
        let ip = alvo.ip();
        let anuncio = partilha.lock().expect("partilha").anuncio.clone();
        let Some(anuncio) = anuncio else { continue };
        let do_quadro = anuncio
            .entradas
            .iter()
            .find(|e| e.id == "quadro_continuo" && e.formato == "rgb_640x360")
            .cloned();
        // A voz só vai se este S1 a guarda.
        let da_voz = cfg.conversa.as_ref().and_then(|_| {
            anuncio
                .entradas
                .iter()
                .find(|e| e.id == "audio_voz" && e.formato == "pcm16_16k")
                .cloned()
        });
        // Para ouvir a fala e para cumprir pedidos, a ligação vale mesmo sem nada a mandar; só com a estação que diz que
        // os manda.
        let ouve_a_fala = (cfg.fala.is_some() || cfg.agente.is_some()) && anuncio.sabe;
        // O que o agente disse desde a última volta vai nesta, ou não vai: desfecho sem estação ligada não é guardado.
        let desfechos: Vec<String> = cfg
            .agente
            .as_ref()
            .map(|a| a.desfechos.try_iter().collect())
            .unwrap_or_default();
        if do_quadro.is_none() && da_voz.is_none() && !ouve_a_fala {
            dizer(
                Some("a estação não pede nenhuma entrada que este S1 manda"),
                Some(ip),
                None,
            );
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        let do_quadro = do_quadro.map(|e| {
            let ritmo = e.ritmo.unwrap_or(cfg.ritmo_max).min(cfg.ritmo_max);
            (e, ritmo)
        });
        let ritmo_vale = do_quadro
            .as_ref()
            .is_none_or(|(_, r)| r.is_finite() && *r > 0.0);
        if !ritmo_vale && da_voz.is_none() && !ouve_a_fala {
            dizer(Some("a estação anunciou ritmo zero"), Some(ip), None);
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        // Com o ritmo em zero e a voz pedida, só a voz vai.
        let do_quadro = do_quadro.filter(|_| ritmo_vale);
        if conexao.as_ref().is_none_or(|(_, a)| *a != alvo) {
            fechar(&mut conexao);
            if agora < tentar_em {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            match TcpStream::connect_timeout(&alvo, Duration::from_secs(1)) {
                Ok(c) => {
                    let _ = c.set_nodelay(true);
                    // A espera só volta ao mínimo depois de a conexão provar que serve ([`QUADROS_DE_PROVA`]): a estação
                    // que recusa o robô aceita a conexão, responde com erro e fecha.
                    na_conexao = 0;
                    // A estação que sabe marcar cada quadro atendido recebe o pedido, uma vez, antes do primeiro quadro: os
                    // dois lados contam os quadros a partir dele. A que não diz que sabe fecharia a conexão ao recebê-lo.
                    {
                        let mut p = partilha.lock().expect("partilha");
                        let pedido = Cabecalho {
                            tipo: Tipo::Atendido,
                            canal: 0,
                            tamanho: 0,
                        };
                        p.marcando = p.anuncio.as_ref().is_some_and(|a| a.atendido)
                            && (&c).write_all(&pedido.bytes()).is_ok();
                        p.em_voo.clear();
                        p.proximo = 0;
                        p.frente_medida = false;
                    }
                    // A estação que diz que sabe mandar o resultado de um pedido recebe, uma vez, o que este S1 sabe receber.
                    // A que não diz fecharia a conexão ao receber o aviso; e sem ele ela não manda nada de novo.
                    let recebe: Vec<&str> = [
                        cfg.conversa.is_some().then_some("resultado"),
                        cfg.fala.is_some().then_some("fala"),
                        cfg.agente.is_some().then_some("pedido"),
                    ]
                    .into_iter()
                    .flatten()
                    .collect();
                    if !recebe.is_empty() && anuncio.sabe {
                        let aviso = serde_json::json!({"recebe": recebe}).to_string();
                        let aviso = aviso.as_bytes();
                        let cab = Cabecalho {
                            tipo: Tipo::Sabe,
                            canal: 0,
                            tamanho: aviso.len() as u32,
                        };
                        let _ = (&c)
                            .write_all(&cab.bytes())
                            .and_then(|()| (&c).write_all(aviso));
                    }
                    if let Ok(leitura) = c.try_clone() {
                        let (p, e, pub_) = (partilha.clone(), estado.clone(), publicar.clone());
                        let conversar = cfg.conversa.as_ref().map(|c| c.publicar.clone());
                        let falar = cfg.fala.clone();
                        let cumprir = cfg
                            .agente
                            .as_ref()
                            .map(|a| (a.pedir.clone(), a.devolver.clone()));
                        std::thread::spawn(move || {
                            ler_entregas(leitura, p, e, pub_, conversar, falar, cumprir)
                        });
                    }
                    conexao = Some((c, alvo));
                    fluxo = None; // numa conexão nova, o H.264 começa por um quadro inteiro
                    proximo = agora;
                    tracing::info!(%alvo, "ligado à estação Hailo");
                }
                Err(e) => {
                    dizer(
                        Some("a estação se anuncia, e a porta dos quadros não responde"),
                        Some(ip),
                        None,
                    );
                    tracing::debug!(%alvo, erro = %e, "sem conexão com a estação");
                    tentar_em = agora + espera;
                    espera = (espera * 2).min(ESPERA_MAX);
                    continue;
                }
            }
        }
        if !desfechos.is_empty() {
            let (c, _) = conexao.as_mut().expect("conexão aberta acima");
            let mut caiu = false;
            for desfecho in &desfechos {
                let cab = Cabecalho {
                    tipo: Tipo::Desfecho,
                    canal: 0,
                    tamanho: desfecho.len() as u32,
                };
                if c.write_all(&cab.bytes())
                    .and_then(|()| c.write_all(desfecho.as_bytes()))
                    .is_err()
                {
                    caiu = true;
                    break;
                }
                estado.lock().expect("estado da percepção").outcomes += 1;
            }
            if caiu {
                fechar(&mut conexao);
                tentar_em = Instant::now() + espera;
                espera = (espera * 2).min(ESPERA_MAX);
                continue;
            }
        }
        if let Some(entrada) = &da_voz {
            let (c, _) = conexao.as_mut().expect("conexão aberta acima");
            let mut caiu = false;
            for trecho in &trechos {
                let pcm: Vec<u8> = trecho.iter().flat_map(|a| a.to_le_bytes()).collect();
                let cab = Cabecalho {
                    tipo: Tipo::Entrada,
                    canal: entrada.n,
                    tamanho: pcm.len() as u32,
                };
                if c.write_all(&cab.bytes())
                    .and_then(|()| c.write_all(&pcm))
                    .is_err()
                {
                    caiu = true;
                    break;
                }
                estado.lock().expect("estado da percepção").voice_sent += 1;
            }
            if caiu {
                fechar(&mut conexao);
                tentar_em = Instant::now() + espera;
                espera = (espera * 2).min(ESPERA_MAX);
                continue;
            }
        }
        let Some((entrada, ritmo)) = do_quadro else {
            // A estação só pede a voz, ou só fala: a ligação fica, e nenhum quadro sai.
            dizer(None, Some(ip), None);
            std::thread::sleep(Duration::from_millis(5));
            continue;
        };
        if agora < proximo {
            std::thread::sleep((proximo - agora).min(Duration::from_millis(5)));
            continue;
        }
        // A estação atrás: este envio espera a marca do quadro mais antigo, sem perder a vez (ele sai assim que ela chegar).
        {
            let mut p = partilha.lock().expect("partilha");
            while p
                .em_voo
                .front()
                .is_some_and(|(_, saiu)| agora.duration_since(*saiu) > SEM_MARCA)
            {
                p.em_voo.pop_front();
                p.frente_medida = false;
            }
            if p.marcando && p.em_voo.len() >= EM_VOO_LIMITE {
                drop(p);
                if !segurando {
                    segurando = true;
                    estado.lock().expect("estado da percepção").frames_held += 1;
                }
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
        }
        segurando = false;
        // O próximo sai um período depois deste, e nunca em rajada para compensar atraso.
        proximo = (proximo + Duration::from_secs_f64(1.0 / ritmo)).max(agora);
        let Some(quadro) = quadros.proximo() else {
            dizer(Some("a câmera não entregou quadro"), Some(ip), Some(ritmo));
            continue;
        };
        // O formato que a estação prefere, se o chip do S1 o faz; senão o cru, com o motivo à vista.
        let (l, a) = (quadro.largura as usize, quadro.altura as usize);
        let mut problema = None;
        let mut codificado = None;
        let prefere = entrada.prefere.as_deref().unwrap_or("cru");
        if let (Some(venc), true) = (&cfg.venc, prefere == "h264" || prefere == "jpeg") {
            let qualidade = entrada.qualidade.unwrap_or(0.0).clamp(0.0, 100.0) as u32;
            match reducao::nv12_reduzido(&quadro.dados, l, a, QUADRO_LARGURA, QUADRO_ALTURA) {
                None => {
                    problema = Some("o quadro da câmera não é NV12 do tamanho que diz".to_owned())
                }
                Some(nv12) => {
                    let mut v = venc.lock().expect("codificador");
                    let feito = if prefere == "jpeg" {
                        fluxo = None;
                        v.jpeg(&nv12, qualidade).map(|d| (Tipo::EntradaJpeg, d))
                    } else {
                        // O fluxo recomeça quando a taxa ou o ritmo mudam, e quando a estação recusou algo: ela pode ter
                        // perdido o fio. No máximo uma vez por segundo, para um modelo que falha em todo quadro não
                        // custar um quadro inteiro por quadro.
                        let recusou =
                            std::mem::take(&mut partilha.lock().expect("partilha").recusou);
                        let este = (qualidade, ritmo.round().max(1.0) as u32);
                        let velho = fluxo.is_some_and(|(q, r, quando): (u32, u32, Instant)| {
                            (q, r) == este
                                && !(recusou && quando.elapsed() >= Duration::from_secs(1))
                        });
                        let aberto = if velho {
                            Ok(())
                        } else {
                            v.fluxo(este.0, este.1)
                                .map(|()| fluxo = Some((este.0, este.1, Instant::now())))
                        };
                        aberto
                            .and_then(|()| v.h264(&nv12))
                            .map(|d| (Tipo::EntradaH264, d))
                    };
                    match feito {
                        Ok(par) => codificado = Some(par),
                        Err(e) => {
                            fluxo = None;
                            problema = Some(e);
                        }
                    }
                }
            }
        } else if prefere == "h264" || prefere == "jpeg" {
            problema = Some("este S1 não tem o codificador ligado ([midia] venc)".to_owned());
        }
        let (tipo, carga, formato) = match codificado {
            Some((Tipo::EntradaJpeg, d)) => (Tipo::EntradaJpeg, d, "jpeg"),
            Some((t, d)) => (t, d, "h264"),
            None => {
                let Some(rgb) =
                    reducao::nv12_para_rgb(&quadro.dados, l, a, QUADRO_LARGURA, QUADRO_ALTURA)
                else {
                    dizer(
                        Some("o quadro da câmera não é NV12 do tamanho que diz"),
                        Some(ip),
                        Some(ritmo),
                    );
                    continue;
                };
                (Tipo::Entrada, rgb, "rgb")
            }
        };
        if problema != ultimo_problema {
            if let Some(p) = &problema {
                tracing::warn!(motivo = %p, "o quadro vai cru, e a estação pediu {prefere}");
            }
            ultimo_problema = problema.clone();
        }
        {
            let mut e = estado.lock().expect("estado da percepção");
            e.format = Some(formato.to_owned());
            e.encoder_problem = problema;
        }
        let rgb = carga;
        let cab = Cabecalho {
            tipo,
            canal: entrada.n,
            tamanho: rgb.len() as u32,
        };
        let (c, _) = conexao.as_mut().expect("conexão aberta acima");
        {
            let mut p = partilha.lock().expect("partilha");
            let agora = Instant::now();
            p.ultimo_envio = Some((agora, quadro.largura, quadro.altura));
            p.medido = false;
            if p.marcando {
                if p.em_voo.len() >= EM_VOO_MAX {
                    p.em_voo.pop_front();
                    p.frente_medida = false;
                }
                let n = p.proximo;
                p.em_voo.push_back((n, agora));
                p.proximo = n.wrapping_add(1);
            }
        }
        if c.write_all(&cab.bytes())
            .and_then(|()| c.write_all(&rgb))
            .is_err()
        {
            fechar(&mut conexao);
            tentar_em = Instant::now() + espera;
            espera = (espera * 2).min(ESPERA_MAX);
            continue;
        }
        na_conexao += 1;
        if na_conexao == QUADROS_DE_PROVA {
            espera = ESPERA_MIN;
        }
        dizer(None, Some(ip), Some(ritmo));
        estado.lock().expect("estado da percepção").frames_sent += 1;
        no_segundo += 1;
        // Uma vez por segundo, o que só o S1 mede: a ida e volta, e quantos quadros mandou.
        if medida_em.elapsed() >= Duration::from_secs(1) {
            let ida = estado.lock().expect("estado da percepção").round_trip_ms;
            let medida = serde_json::json!({"ida_e_volta_ms": ida, "quadros_por_s": f64::from(no_segundo) / medida_em.elapsed().as_secs_f64()}).to_string();
            let cab = Cabecalho {
                tipo: Tipo::Medida,
                canal: 0,
                tamanho: medida.len() as u32,
            };
            if c.write_all(&cab.bytes())
                .and_then(|()| c.write_all(medida.as_bytes()))
                .is_err()
            {
                fechar(&mut conexao);
            }
            medida_em = Instant::now();
            no_segundo = 0;
        }
    }
    Ok(())
}

/// Fecha a conexão dos dois lados: quem lê as entregas tem uma cópia dela, e só assim sai da leitura.
fn fechar(conexao: &mut Option<(TcpStream, SocketAddr)>) {
    if let Some((c, _)) = conexao.take() {
        let _ = c.shutdown(std::net::Shutdown::Both);
    }
}

/// Abre a porta do anúncio para ser dividida: no Mac, o serviço do simulador ouve a mesma.
fn ouvir(porta: u16) -> std::io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    s.set_reuse_port(true)?;
    s.bind(&SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), porta).into())?;
    let s: UdpSocket = s.into();
    s.set_nonblocking(true)?;
    Ok(s)
}

/// Quem recebe o pedido a cumprir, e por onde volta a recusa do que ninguém recebeu.
type Cumprir = (
    Arc<dyn Fn(String) -> bool + Send + Sync>,
    std::sync::mpsc::Sender<String>,
);

/// Lê o que a estação manda, até a conexão fechar.
fn ler_entregas(
    mut conexao: TcpStream,
    partilha: Arc<Mutex<Partilha>>,
    estado: Arc<Mutex<Estado>>,
    publicar: Arc<impl Fn(String) + Send + Sync + 'static>,
    conversar: Option<Arc<dyn Fn(String) + Send + Sync>>,
    falar: Option<Arc<dyn Fn(Som) + Send + Sync>>,
    cumprir: Option<Cumprir>,
) {
    // A conexão que cai no meio de uma fala não deixa o alto-falante à espera do resto: o que chegou toca até o fim.
    struct FalaEmCurso(bool, Option<Arc<dyn Fn(Som) + Send + Sync>>);
    impl Drop for FalaEmCurso {
        fn drop(&mut self) {
            if let (true, Some(falar)) = (self.0, &self.1) {
                falar(Som::Fim {
                    interrompida: false,
                });
            }
        }
    }
    let mut em_curso = FalaEmCurso(false, falar.clone());
    let mut cab = [0u8; Cabecalho::BYTES];
    loop {
        if conexao.read_exact(&mut cab).is_err() {
            return;
        }
        let c = match Cabecalho::ler(&cab) {
            Ok(c) => c,
            Err(e) => {
                // Sem saber o tamanho da carga, não há como achar a próxima mensagem: a conexão recomeça.
                tracing::warn!(erro = e, "a estação mandou um cabeçalho que este S1 não lê");
                estado.lock().expect("estado da percepção").errors += 1;
                let _ = conexao.shutdown(std::net::Shutdown::Both);
                return;
            }
        };
        let mut carga = vec![0u8; c.tamanho as usize];
        if conexao.read_exact(&mut carga).is_err() {
            return;
        }
        match c.tipo {
            Tipo::Entrega => {
                let (funcao, tamanho, saiu_em) = {
                    let mut p = partilha.lock().expect("partilha");
                    let f = p
                        .anuncio
                        .as_ref()
                        .and_then(|a| a.funcoes.iter().find(|f| f.n == c.canal).cloned());
                    // A ida e volta é a da primeira entrega de um quadro. Com a estação marcando cada quadro atendido, a
                    // entrega é do quadro mais antigo ainda não marcado; sem isso, supõe-se que é do último envio.
                    let saiu_em = if p.marcando {
                        match p.em_voo.front().map(|(_, quando)| *quando) {
                            Some(quando) if !p.frente_medida => {
                                p.frente_medida = true;
                                Some(quando)
                            }
                            _ => None,
                        }
                    } else if !std::mem::replace(&mut p.medido, true) {
                        p.ultimo_envio.map(|(quando, _, _)| quando)
                    } else {
                        None
                    };
                    (f, p.ultimo_envio.map(|(_, l, a)| (l, a)), saiu_em)
                };
                // Entrega de uma função que o anúncio não tem, ou sem quadro mandado: não há como ler.
                let (Some(funcao), Some((l, a))) = (funcao, tamanho) else {
                    estado.lock().expect("estado da percepção").errors += 1;
                    continue;
                };
                let levou = {
                    let mut e = estado.lock().expect("estado da percepção");
                    *e.deliveries.entry(funcao.id.clone()).or_default() += 1;
                    if let Some(quando) = saiu_em {
                        e.round_trip_ms = Some(quando.elapsed().as_secs_f64() * 1000.0);
                    }
                    e.round_trip_ms.unwrap_or(0.0)
                };
                if funcao.entrega == "caixas" && funcao.leitor.as_deref() == Some("nanodet_v8") {
                    let nota = funcao.nota.unwrap_or(0.35) as f32;
                    let iou = funcao.iou.unwrap_or(0.5) as f32;
                    match deteccoes(&carga, l, a, nota, iou) {
                        // Só a função que pede é publicada; as outras são só contadas.
                        Ok(achadas) if funcao.publica.as_deref() == Some("media.detections") => {
                            publicar(notificacao(&achadas, l, a, levou))
                        }
                        Ok(_) => {}
                        Err(erro) => {
                            tracing::warn!(erro, funcao = funcao.id, "entrega que este S1 não leu");
                            estado.lock().expect("estado da percepção").errors += 1;
                        }
                    }
                } else if funcao.entrega == "caixas"
                    && funcao.leitor.as_deref() == Some("caixas_padrao_hailo")
                {
                    let nota = funcao.nota.unwrap_or(0.35) as f32;
                    match deteccoes_com_classe(&carga, l, a, nota) {
                        Ok(achadas) if funcao.publica.as_deref() == Some("media.detections") => {
                            publicar(notificacao_com_classe(&achadas, l, a, levou))
                        }
                        Ok(_) => {}
                        Err(erro) => {
                            tracing::warn!(erro, funcao = funcao.id, "entrega que este S1 não leu");
                            estado.lock().expect("estado da percepção").errors += 1;
                        }
                    }
                }
            }
            Tipo::Erro => {
                tracing::warn!(canal = c.canal, motivo = %String::from_utf8_lossy(&carga), "a estação recusou uma mensagem");
                estado.lock().expect("estado da percepção").errors += 1;
                partilha.lock().expect("partilha").recusou = true;
            }
            Tipo::Atendido => {
                // O quadro marcado sai da fila, com os mais antigos que ele: as entregas seguintes são do quadro seguinte.
                let mut p = partilha.lock().expect("partilha");
                if let Some(i) = p.em_voo.iter().position(|(n, _)| *n == c.canal) {
                    p.em_voo.drain(..=i);
                    p.frente_medida = false;
                }
            }
            Tipo::Resultado => {
                // Um acontecimento de um pedido que a voz deste S1 começou: vai a quem assina a conversa, com a função e o
                // tipo do que ela entrega, que é o que o anúncio diz dela.
                let funcao = partilha
                    .lock()
                    .expect("partilha")
                    .anuncio
                    .as_ref()
                    .and_then(|a| a.funcoes.iter().find(|f| f.n == c.canal).cloned());
                let acontecimento = serde_json::from_slice::<serde_json::Value>(&carga)
                    .ok()
                    .filter(serde_json::Value::is_object);
                match (funcao, acontecimento, &conversar) {
                    (Some(funcao), Some(acontecimento), Some(conversar)) => {
                        estado.lock().expect("estado da percepção").results += 1;
                        conversar(
                            serde_json::json!({"jsonrpc": "2.0", "method": "s1.station.result", "params": {"function": funcao.id, "delivers": funcao.entrega, "event": acontecimento}})
                                .to_string(),
                        );
                    }
                    _ => estado.lock().expect("estado da percepção").errors += 1,
                }
            }
            Tipo::Fala => match &falar {
                Some(falar) if carga.is_empty() => {
                    em_curso.0 = false;
                    falar(Som::Fim {
                        interrompida: c.canal == 1,
                    });
                }
                Some(falar) => {
                    em_curso.0 = true;
                    estado.lock().expect("estado da percepção").speech_received += 1;
                    falar(Som::Pedaco {
                        hz: u32::from(c.canal),
                        pcm: carga,
                    });
                }
                // Este S1 não disse que recebe a fala: é erro de quem mandou.
                None => estado.lock().expect("estado da percepção").errors += 1,
            },
            Tipo::Pedido => {
                let pedido = serde_json::from_slice::<serde_json::Value>(&carga)
                    .ok()
                    .and_then(|v| {
                        Some((
                            v["pedido"].as_u64()?,
                            v["ferramenta"].as_str()?.to_owned(),
                            v,
                        ))
                    });
                match (pedido, &cumprir) {
                    (Some((numero, ferramenta, v)), Some((pedir, devolver))) => {
                        estado.lock().expect("estado da percepção").requests += 1;
                        let params = match v["parametros"].as_object() {
                            Some(p) => serde_json::Value::Object(p.clone()),
                            None => serde_json::json!({}),
                        };
                        let linha = serde_json::json!({"jsonrpc": "2.0", "method": "s1.station.request", "params": {"request": numero, "tool": ferramenta, "params": params}});
                        if !pedir(linha.to_string()) {
                            // Ninguém assina os pedidos: a recusa volta já, pelo mesmo caminho dos desfechos.
                            let recusa = serde_json::json!({"pedido": numero, "ferramenta": ferramenta, "como": "recusada", "motivo": "nenhum agente ligado ao rms1-mediad"});
                            let _ = devolver.send(recusa.to_string());
                        }
                    }
                    // O pedido que não se lê, ou a um S1 que não disse que cumpre: é erro de quem mandou.
                    _ => estado.lock().expect("estado da percepção").errors += 1,
                }
            }
            // A estação não manda entradas, medidas, o aviso do que sabe nem desfechos.
            Tipo::Entrada
            | Tipo::EntradaH264
            | Tipo::EntradaJpeg
            | Tipo::Medida
            | Tipo::Sabe
            | Tipo::Desfecho => estado.lock().expect("estado da percepção").errors += 1,
        }
    }
}
