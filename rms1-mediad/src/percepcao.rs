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

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use duck_detect::{Detection, Letterbox};

use crate::estacao::{self, Anuncio, Cabecalho, Tipo};
use crate::quadros::Quadros;
use crate::reducao;

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
}

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
    let mut pacote = [0u8; 65535];
    let dizer = |por_que: Option<&str>, estacao: Option<IpAddr>, ritmo: Option<f64>| {
        let mut e = estado.lock().expect("estado da percepção");
        e.idle_reason = por_que.map(str::to_owned);
        e.station = estacao.map(|ip| ip.to_string());
        e.rate = ritmo;
    };

    while !parar.load(Ordering::Relaxed) {
        let agora = Instant::now();
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
        let Some(entrada) = anuncio
            .entradas
            .iter()
            .find(|e| e.id == "quadro_continuo" && e.formato == "rgb_640x360")
        else {
            dizer(
                Some("a estação não pede nenhuma entrada que este S1 manda"),
                Some(ip),
                None,
            );
            std::thread::sleep(Duration::from_millis(20));
            continue;
        };
        let ritmo = entrada.ritmo.unwrap_or(cfg.ritmo_max).min(cfg.ritmo_max);
        if !(ritmo.is_finite() && ritmo > 0.0) {
            dizer(Some("a estação anunciou ritmo zero"), Some(ip), None);
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
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
                    if let Ok(leitura) = c.try_clone() {
                        let (p, e, pub_) = (partilha.clone(), estado.clone(), publicar.clone());
                        std::thread::spawn(move || ler_entregas(leitura, p, e, pub_));
                    }
                    conexao = Some((c, alvo));
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
        if agora < proximo {
            std::thread::sleep((proximo - agora).min(Duration::from_millis(5)));
            continue;
        }
        // O próximo sai um período depois deste, e nunca em rajada para compensar atraso.
        proximo = (proximo + Duration::from_secs_f64(1.0 / ritmo)).max(agora);
        let Some(quadro) = quadros.proximo() else {
            dizer(Some("a câmera não entregou quadro"), Some(ip), Some(ritmo));
            continue;
        };
        let Some(rgb) = reducao::nv12_para_rgb(
            &quadro.dados,
            quadro.largura as usize,
            quadro.altura as usize,
            QUADRO_LARGURA,
            QUADRO_ALTURA,
        ) else {
            dizer(
                Some("o quadro da câmera não é NV12 do tamanho que diz"),
                Some(ip),
                Some(ritmo),
            );
            continue;
        };
        let cab = Cabecalho {
            tipo: Tipo::Entrada,
            canal: entrada.n,
            tamanho: rgb.len() as u32,
        };
        let (c, _) = conexao.as_mut().expect("conexão aberta acima");
        {
            let mut p = partilha.lock().expect("partilha");
            p.ultimo_envio = Some((Instant::now(), quadro.largura, quadro.altura));
            p.medido = false;
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

/// Lê o que a estação manda, até a conexão fechar.
fn ler_entregas(
    mut conexao: TcpStream,
    partilha: Arc<Mutex<Partilha>>,
    estado: Arc<Mutex<Estado>>,
    publicar: Arc<impl Fn(String) + Send + Sync + 'static>,
) {
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
                let (funcao, envio, medir) = {
                    let mut p = partilha.lock().expect("partilha");
                    let f = p
                        .anuncio
                        .as_ref()
                        .and_then(|a| a.funcoes.iter().find(|f| f.n == c.canal).cloned());
                    // A primeira entrega depois de um envio é a medida da ida e volta desse quadro.
                    let medir = !std::mem::replace(&mut p.medido, true);
                    (f, p.ultimo_envio, medir)
                };
                // Entrega de uma função que o anúncio não tem, ou sem quadro mandado: não há como ler.
                let (Some(funcao), Some((quando, l, a))) = (funcao, envio) else {
                    estado.lock().expect("estado da percepção").errors += 1;
                    continue;
                };
                let levou = {
                    let mut e = estado.lock().expect("estado da percepção");
                    *e.deliveries.entry(funcao.id.clone()).or_default() += 1;
                    if medir {
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
            }
            // A estação não manda entradas nem medidas ao S1.
            Tipo::Entrada | Tipo::Medida => estado.lock().expect("estado da percepção").errors += 1,
        }
    }
}
