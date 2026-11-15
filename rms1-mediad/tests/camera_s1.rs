//! A câmera do robô contra um dispositivo de mentira que responde como a gravação da DJI
//! (`out/2026-10-30-poc2/camera/dji_camera_v4l2_decoded.strace`, no laboratório). Os números da gravação estão aqui por
//! extenso, e não vêm das constantes do módulo: uma constante plantada errada faz o módulo e a gravação discordarem.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rms1_mediad::camera_s1::{self as cam, CameraS1, Dispositivo, Erro, Mapa, Passo};
use rms1_mediad::quadros::Quadros;

/// O que o dispositivo viu, na ordem: um pedido com os bytes que chegaram, um mapa, um mapa solto.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ato {
    Pedido(u32, Vec<u8>),
    Mapa(usize, u32),
    Solto(u32),
}

/// As respostas da gravação, com uma delas torta para os testes de divergência.
#[derive(Clone)]
struct Respostas {
    driver: &'static str,
    segunda_entrada: bool,
    entrada_lida: u32,
    eco_do_controle: Option<u32>,
    tamanho_da_imagem: u32,
    quantos_buffers: u32,
    tamanho_do_buffer: u32,
    limites: (u32, u32),
    bytes_do_quadro: u32,
    /// O número do quadro em que tirar da fila passa a dar erro.
    falha_no_quadro: Option<u32>,
    /// O número do quadro a partir do qual o driver pula três na numeração.
    salto_no_quadro: Option<u32>,
}

impl Default for Respostas {
    fn default() -> Self {
        Self {
            driver: "comip",
            segunda_entrada: false,
            entrada_lida: 1,
            eco_do_controle: None,
            tamanho_da_imagem: 1_382_432,
            quantos_buffers: 5,
            tamanho_do_buffer: 1_384_448,
            limites: (2592, 1944),
            bytes_do_quadro: 1_382_432,
            falha_no_quadro: None,
            salto_no_quadro: None,
        }
    }
}

struct Gravacao {
    r: Respostas,
    atos: Arc<Mutex<Vec<Ato>>>,
    fila: Vec<u32>,
    ligado: bool,
    quadros: u32,
}

impl Gravacao {
    fn nova(r: Respostas) -> (Self, Arc<Mutex<Vec<Ato>>>) {
        let atos = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                r,
                atos: atos.clone(),
                fila: Vec::new(),
                ligado: false,
                quadros: 0,
            },
            atos,
        )
    }
}

fn le(b: &[u8], em: usize) -> u32 {
    u32::from_le_bytes(b[em..em + 4].try_into().unwrap())
}

fn poe(b: &mut [u8], em: usize, v: u32) {
    b[em..em + 4].copy_from_slice(&v.to_le_bytes());
}

fn einval() -> io::Error {
    io::Error::from_raw_os_error(22)
}

impl Dispositivo for Gravacao {
    fn ioctl(&mut self, pedido: u32, arg: &mut [u8]) -> io::Result<()> {
        self.atos
            .lock()
            .unwrap()
            .push(Ato::Pedido(pedido, arg.to_vec()));
        match pedido {
            // VIDIOC_QUERYCAP: driver="comip", card="comip", bus_info="comip-camera", 1.0.0, captura e fluxo.
            0x8068_5600 => {
                arg[..self.r.driver.len()].copy_from_slice(self.r.driver.as_bytes());
                arg[16..21].copy_from_slice(b"comip");
                arg[48..60].copy_from_slice(b"comip-camera");
                poe(arg, 80, 0x0001_0000);
                poe(arg, 84, 0x0400_0001);
            }
            // VIDIOC_ENUMINPUT: a 0 é "ov5695", uma câmera; a 1 dá EINVAL.
            0xc050_561a => match le(arg, 0) {
                0 => {
                    arg[4..10].copy_from_slice(b"ov5695");
                    poe(arg, 36, 2);
                }
                1 if self.r.segunda_entrada => {
                    arg[4..9].copy_from_slice(b"outra");
                    poe(arg, 36, 2);
                }
                _ => return Err(einval()),
            },
            // VIDIOC_S_INPUT [0]
            0xc004_5627 => assert_eq!(le(arg, 0), 0),
            // VIDIOC_CROPCAP: limites e padrão 2592x1944, 1/1.
            0xc02c_563a => {
                for em in [12, 28] {
                    poe(arg, em, self.r.limites.0);
                    poe(arg, em + 4, self.r.limites.1);
                }
                poe(arg, 36, 1);
                poe(arg, 40, 1);
            }
            // VIDIOC_G_INPUT [1]
            0x8004_5626 => poe(arg, 0, self.r.entrada_lida),
            // VIDIOC_S_CTRL: o valor volta como foi.
            0xc008_561c => {
                if let Some(v) = self.r.eco_do_controle {
                    poe(arg, 4, v);
                }
            }
            // VIDIOC_S_FMT: campo entrelaçado, 1920 bytes por linha, imagem de 1.382.432.
            0xc0cc_5605 => {
                poe(arg, 16, 4);
                poe(arg, 20, 1920);
                poe(arg, 24, self.r.tamanho_da_imagem);
            }
            // VIDIOC_REQBUFS: 5 => 5
            0xc014_5608 => poe(arg, 0, self.r.quantos_buffers),
            // VIDIOC_QUERYBUF: cada buffer com 1.384.448 bytes, um depois do outro.
            0xc044_5609 => {
                let i = le(arg, 0);
                poe(arg, 52, i * 1_384_448);
                poe(arg, 56, self.r.tamanho_do_buffer);
            }
            // VIDIOC_QBUF
            0xc044_560f => self.fila.push(le(arg, 0)),
            // VIDIOC_DQBUF
            0xc044_5611 => {
                assert!(self.ligado, "tirou da fila antes de ligar o fluxo");
                if self.r.falha_no_quadro == Some(self.quadros) {
                    return Err(io::Error::from_raw_os_error(19));
                }
                let i = self.fila.remove(0);
                poe(arg, 0, i);
                poe(arg, 8, self.r.bytes_do_quadro);
                let pulo = if self.r.salto_no_quadro.is_some_and(|n| self.quadros >= n) {
                    3
                } else {
                    0
                };
                poe(arg, 44, self.quadros + pulo);
                self.quadros += 1;
            }
            // VIDIOC_STREAMON e VIDIOC_STREAMOFF
            0x4004_5612 => self.ligado = true,
            0x4004_5613 => self.ligado = false,
            outro => panic!("a DJI nunca mandou o pedido 0x{outro:08x}"),
        }
        Ok(())
    }

    fn mapear(&mut self, tamanho: usize, deslocamento: u32) -> io::Result<Mapa> {
        self.atos
            .lock()
            .unwrap()
            .push(Ato::Mapa(tamanho, deslocamento));
        let (i, atos) = (deslocamento / 1_384_448, self.atos.clone());
        // Cada buffer com um byte próprio, para o teste saber de qual o quadro saiu.
        Ok(Mapa::proprio(
            tamanho,
            i as u8 + 1,
            Box::new(move || atos.lock().unwrap().push(Ato::Solto(i))),
        ))
    }

    fn esperar(&mut self, _ms: i32) -> io::Result<bool> {
        std::thread::sleep(Duration::from_millis(1));
        Ok(self.ligado && !self.fila.is_empty())
    }
}

/// Os bytes de um pedido: `n` zeros com os campos dados, em pares de deslocamento e valor.
fn bytes(n: usize, campos: &[(usize, u32)]) -> Vec<u8> {
    let mut b = vec![0u8; n];
    for (em, v) in campos {
        poe(&mut b, *em, *v);
    }
    b
}

/// Os 27 passos da DJI, como a gravação os mostra: o pedido e os bytes mandados, e os cinco mapas.
fn gravacao_da_dji() -> Vec<Ato> {
    let mut g = vec![
        Ato::Pedido(0x8068_5600, bytes(104, &[])),
        Ato::Pedido(0xc050_561a, bytes(80, &[])),
        Ato::Pedido(0xc050_561a, bytes(80, &[(0, 1)])),
        Ato::Pedido(0xc004_5627, bytes(4, &[])),
        Ato::Pedido(0xc02c_563a, bytes(44, &[])),
        Ato::Pedido(0x8004_5626, bytes(4, &[])),
        Ato::Pedido(0xc008_561c, bytes(8, &[(0, 0x009a_191b), (4, 0)])),
        Ato::Pedido(0xc008_561c, bytes(8, &[(0, 0x009a_1934), (4, 30)])),
        Ato::Pedido(0xc008_561c, bytes(8, &[(0, 0x009a_1935), (4, 0)])),
        Ato::Pedido(
            0xc0cc_5605,
            bytes(
                204,
                &[
                    (0, 1),
                    (4, 1280),
                    (8, 720),
                    (12, 0x3231_564e),
                    (16, 0),
                    (28, 7),
                ],
            ),
        ),
        Ato::Pedido(0xc014_5608, bytes(20, &[(0, 5), (4, 1), (8, 1)])),
    ];
    for i in 0..5u32 {
        g.push(Ato::Pedido(
            0xc044_5609,
            bytes(68, &[(0, i), (4, 1), (48, 1)]),
        ));
        g.push(Ato::Mapa(1_384_448, i * 1_384_448));
    }
    for i in 0..5u32 {
        g.push(Ato::Pedido(
            0xc044_560f,
            bytes(68, &[(0, i), (4, 1), (48, 1)]),
        ));
    }
    g.push(Ato::Pedido(0x4004_5612, bytes(4, &[(0, 1)])));
    g
}

const LIGAR: u32 = 0x4004_5612;
const DESLIGAR: u32 = 0x4004_5613;

fn desligou_e_soltou(atos: &[Ato]) {
    let n = atos.len();
    assert!(n >= 6, "{atos:?}");
    assert_eq!(
        atos[n - 6],
        Ato::Pedido(DESLIGAR, bytes(4, &[(0, 1)])),
        "o fluxo é desligado antes de soltar"
    );
    let mut soltos: Vec<u32> = atos[n - 5..]
        .iter()
        .map(|a| match a {
            Ato::Solto(i) => *i,
            outro => panic!("depois de desligar só se soltam os buffers: {outro:?}"),
        })
        .collect();
    soltos.sort_unstable();
    assert_eq!(soltos, vec![0, 1, 2, 3, 4]);
    assert!(
        !atos[..n - 6].iter().any(|a| matches!(a, Ato::Solto(_))),
        "nenhum buffer é solto com o fluxo ligado"
    );
}

#[test]
fn camera_do_s1_repete_a_gravacao_da_dji() {
    assert_eq!(cam::PASSOS.len(), 27);
    let (dev, atos) = Gravacao::nova(Respostas::default());
    let mut camera = CameraS1::abrir(dev).expect("a gravação bate com ela mesma");
    assert_eq!(
        *atos.lock().unwrap(),
        gravacao_da_dji(),
        "os 27 passos, byte a byte"
    );

    // Os quadros saem dos buffers na ordem em que entraram na fila, com os bytes que o driver disse, e voltam à fila.
    for n in 0..7u32 {
        let (indice, tamanho, primeiro) = {
            let (i, b) = camera.quadro(100).expect("sem erro").expect("um quadro");
            (i, b.len(), b[0])
        };
        assert_eq!(
            (indice, tamanho, primeiro),
            (n % 5, 1_382_432, (n % 5) as u8 + 1)
        );
        camera.devolver(indice).unwrap();
    }
    drop(camera);
    desligou_e_soltou(&atos.lock().unwrap());
}

#[test]
fn camera_do_s1_para_antes_do_fluxo_na_divergencia() {
    let casos: Vec<(Respostas, Passo, usize, &str)> = vec![
        (
            Respostas {
                driver: "uvcvideo",
                ..Respostas::default()
            },
            Passo::Consulta,
            1,
            "uvcvideo",
        ),
        (
            Respostas {
                segunda_entrada: true,
                ..Respostas::default()
            },
            Passo::Entrada(1),
            3,
            "outra",
        ),
        (
            Respostas {
                limites: (1920, 1080),
                ..Respostas::default()
            },
            Passo::Recorte,
            5,
            "1920x1080",
        ),
        (
            Respostas {
                entrada_lida: 0,
                ..Respostas::default()
            },
            Passo::LerEntrada,
            6,
            "veio 0",
        ),
        (
            Respostas {
                eco_do_controle: Some(15),
                ..Respostas::default()
            },
            Passo::Controle(0x009a_191b),
            7,
            "veio 15",
        ),
        (
            Respostas {
                tamanho_da_imagem: 1_382_400,
                ..Respostas::default()
            },
            Passo::Formato,
            10,
            "1382400",
        ),
        (
            Respostas {
                quantos_buffers: 4,
                ..Respostas::default()
            },
            Passo::PedirBuffers,
            11,
            "veio 4",
        ),
    ];
    for (r, onde, mandados, trecho) in casos {
        let (dev, atos) = Gravacao::nova(r);
        let erro = CameraS1::abrir(dev).err().expect("divergiu");
        let texto = erro.to_string();
        match erro {
            Erro::Divergiu { passo, .. } => assert_eq!(passo, onde),
            Erro::Sistema { .. } => panic!("{texto}"),
        }
        assert!(
            texto.contains(trecho) && texto.contains("o fluxo não foi ligado"),
            "{texto}"
        );
        let atos = atos.lock().unwrap();
        assert_eq!(
            *atos,
            gravacao_da_dji()[..mandados],
            "{onde:?}: nada depois do passo que divergiu"
        );
        assert!(!atos.iter().any(|a| matches!(a, Ato::Pedido(LIGAR, _))));
    }

    // O buffer de outro tamanho aparece na primeira consulta: nenhum mapa é feito, e o fluxo não liga.
    let (mut dev, atos) = Gravacao::nova(Respostas::default());
    dev.r.tamanho_do_buffer = 1_382_432;
    let erro = CameraS1::abrir(dev).err().expect("divergiu");
    assert!(
        matches!(
            erro,
            Erro::Divergiu {
                passo: Passo::ConsultarBuffer(0),
                ..
            }
        ),
        "{erro}"
    );
    let atos = atos.lock().unwrap();
    assert_eq!(*atos, gravacao_da_dji()[..12]);
}

#[test]
fn camera_do_s1_sempre_desliga_o_fluxo() {
    // Saída comum: quem tem a câmera a solta.
    let (dev, atos) = Gravacao::nova(Respostas::default());
    drop(CameraS1::abrir(dev).unwrap());
    desligou_e_soltou(&atos.lock().unwrap());

    // A captura falha no meio (o `ENODEV` que a câmera deu em 2026-10-30): o laço sai, e desliga.
    let (dev, atos) = Gravacao::nova(Respostas {
        falha_no_quadro: Some(4),
        ..Respostas::default()
    });
    let dito = Arc::new(Mutex::new(Vec::new()));
    let d = dito.clone();
    cam::rodar(
        Quadros::default(),
        dev,
        2,
        move |m| d.lock().unwrap().push(m),
        Arc::new(AtomicBool::new(false)),
    );
    desligou_e_soltou(&atos.lock().unwrap());
    let dito = dito.lock().unwrap();
    assert_eq!(dito[0], None, "aqueceu");
    assert!(
        dito[1]
            .as_deref()
            .is_some_and(|m| m.contains("a captura falhou")),
        "{dito:?}"
    );

    // O quadro curto, que ninguém saberia ler: o laço para, e desliga.
    let (dev, atos) = Gravacao::nova(Respostas {
        bytes_do_quadro: 1000,
        ..Respostas::default()
    });
    cam::rodar(
        Quadros::default(),
        dev,
        0,
        |_| {},
        Arc::new(AtomicBool::new(false)),
    );
    desligou_e_soltou(&atos.lock().unwrap());

    // O serviço manda parar: o laço sai, e desliga.
    let (dev, atos) = Gravacao::nova(Respostas::default());
    let (quadros, parar) = (Quadros::default(), Arc::new(AtomicBool::new(false)));
    let (q, p) = (quadros.clone(), parar.clone());
    let laco = std::thread::spawn(move || cam::rodar(q, dev, 3, |_| {}, p));
    // Sem pedido, nenhum quadro é copiado; com pedido, um, sem os 32 bytes do driver.
    std::thread::sleep(Duration::from_millis(60));
    assert_eq!(quadros.copias(), 0);
    let quadro = quadros.proximo().expect("um quadro da câmera");
    assert_eq!(
        (
            quadro.largura,
            quadro.altura,
            quadro.formato,
            quadro.dados.len()
        ),
        (1280, 720, "NV12", 1_382_400)
    );
    assert!(
        (1..=5).contains(&quadro.dados[0]),
        "o quadro saiu de um buffer do driver"
    );
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(quadros.copias(), 1);
    parar.store(true, Ordering::Relaxed);
    laco.join().unwrap();
    desligou_e_soltou(&atos.lock().unwrap());
}

/// A câmera conta o que entrega e o que o driver pulou: é o que a prova no robô lê para dizer "sem descarte".
#[test]
fn camera_do_s1_conta_os_quadros_e_os_que_o_driver_pulou() {
    let r = Respostas {
        salto_no_quadro: Some(5),
        falha_no_quadro: Some(9),
        ..Respostas::default()
    };
    let (dev, _atos) = Gravacao::nova(r);
    let contagem = cam::Contagem::default();
    cam::rodar_contando(
        Quadros::default(),
        dev,
        2,
        |_| {},
        &contagem,
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(
        contagem.quadros.load(Ordering::Relaxed),
        9,
        "nove quadros antes da falha"
    );
    assert_eq!(
        contagem.perdidos.load(Ordering::Relaxed),
        3,
        "o salto de três na numeração"
    );
    // Sem salto, nenhum perdido.
    let (dev, _atos) = Gravacao::nova(Respostas {
        falha_no_quadro: Some(9),
        ..Respostas::default()
    });
    let contagem = cam::Contagem::default();
    cam::rodar_contando(
        Quadros::default(),
        dev,
        2,
        |_| {},
        &contagem,
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(
        (
            contagem.quadros.load(Ordering::Relaxed),
            contagem.perdidos.load(Ordering::Relaxed)
        ),
        (9, 0)
    );
}

/// Cada número de pedido e cada deslocamento usado, conferido com o que o compilador ARM de 32 bits imprimiu.
#[test]
fn os_numeros_sao_os_do_compilador_arm() {
    let texto = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../s1d/tests/fixtures/v4l2_arm32.json"
    ))
    .unwrap();
    let o: serde_json::Value = serde_json::from_str(&texto).unwrap();
    assert_eq!(o["arch"], "arm32");
    let pares: &[(&str, u64)] = &[
        ("VIDIOC_QUERYCAP", cam::VIDIOC_QUERYCAP.into()),
        ("VIDIOC_ENUMINPUT", cam::VIDIOC_ENUMINPUT.into()),
        ("VIDIOC_G_INPUT", cam::VIDIOC_G_INPUT.into()),
        ("VIDIOC_S_INPUT", cam::VIDIOC_S_INPUT.into()),
        ("VIDIOC_CROPCAP", cam::VIDIOC_CROPCAP.into()),
        ("VIDIOC_S_CTRL", cam::VIDIOC_S_CTRL.into()),
        ("VIDIOC_S_FMT", cam::VIDIOC_S_FMT.into()),
        ("VIDIOC_REQBUFS", cam::VIDIOC_REQBUFS.into()),
        ("VIDIOC_QUERYBUF", cam::VIDIOC_QUERYBUF.into()),
        ("VIDIOC_QBUF", cam::VIDIOC_QBUF.into()),
        ("VIDIOC_DQBUF", cam::VIDIOC_DQBUF.into()),
        ("VIDIOC_STREAMON", cam::VIDIOC_STREAMON.into()),
        ("VIDIOC_STREAMOFF", cam::VIDIOC_STREAMOFF.into()),
        ("V4L2_BUF_TYPE_VIDEO_CAPTURE", cam::CAPTURA.into()),
        ("V4L2_MEMORY_MMAP", cam::MMAP.into()),
        ("V4L2_PIX_FMT_NV12", cam::FOURCC_NV12.into()),
        ("V4L2_FIELD_ANY", cam::CAMPO_QUALQUER.into()),
        ("V4L2_FIELD_INTERLACED", cam::CAMPO_ENTRELACADO.into()),
        ("V4L2_COLORSPACE_JPEG", cam::CORES_JPEG.into()),
        ("V4L2_CTRL_CLASS_CAMERA", cam::CLASSE_CAMERA.into()),
        ("V4L2_CAP_VIDEO_CAPTURE", cam::PODE_CAPTURAR.into()),
        ("V4L2_CAP_STREAMING", cam::PODE_FLUXO.into()),
        ("V4L2_INPUT_TYPE_CAMERA", cam::ENTRADA_CAMERA.into()),
        ("off_v4l2_capability_driver", cam::CAP_DRIVER as u64),
        (
            "off_v4l2_capability_capabilities",
            cam::CAP_CAPACIDADES as u64,
        ),
        ("off_v4l2_input_index", cam::ENT_INDICE as u64),
        ("off_v4l2_input_name", cam::ENT_NOME as u64),
        ("off_v4l2_input_type", cam::ENT_TIPO as u64),
        ("off_v4l2_control_id", cam::CTL_ID as u64),
        ("off_v4l2_control_value", cam::CTL_VALOR as u64),
        ("off_v4l2_format_type", cam::FMT_TIPO as u64),
        ("off_v4l2_requestbuffers_count", cam::PED_QUANTOS as u64),
        ("off_v4l2_requestbuffers_type", cam::PED_TIPO as u64),
        ("off_v4l2_requestbuffers_memory", cam::PED_MEMORIA as u64),
        ("off_v4l2_buffer_index", cam::BUF_INDICE as u64),
        ("off_v4l2_buffer_type", cam::BUF_TIPO as u64),
        ("off_v4l2_buffer_bytesused", cam::BUF_USADOS as u64),
        ("off_v4l2_buffer_sequence", cam::BUF_SEQUENCIA as u64),
        ("off_v4l2_buffer_memory", cam::BUF_MEMORIA as u64),
        ("off_v4l2_buffer_m", cam::BUF_DESLOCAMENTO as u64),
        ("off_v4l2_buffer_length", cam::BUF_TAMANHO as u64),
    ];
    for (chave, nosso) in pares {
        assert_eq!(o[*chave].as_u64(), Some(*nosso), "{chave}");
    }
    // Os campos do formato e do recorte ficam dentro de estruturas aninhadas: a soma dos dois deslocamentos.
    let soma = |a: &str, b: &str| o[a].as_u64().unwrap() + o[b].as_u64().unwrap();
    let fmt = "off_v4l2_format_fmt";
    for (campo, nosso) in [
        ("off_v4l2_pix_format_width", cam::FMT_LARGURA),
        ("off_v4l2_pix_format_height", cam::FMT_ALTURA),
        ("off_v4l2_pix_format_pixelformat", cam::FMT_FOURCC),
        ("off_v4l2_pix_format_field", cam::FMT_CAMPO),
        ("off_v4l2_pix_format_bytesperline", cam::FMT_BYTES_POR_LINHA),
        ("off_v4l2_pix_format_sizeimage", cam::FMT_TAMANHO),
        ("off_v4l2_pix_format_colorspace", cam::FMT_CORES),
    ] {
        assert_eq!(soma(fmt, campo), nosso as u64, "{campo}");
    }
    // `bounds` é um `v4l2_rect`: esquerda, topo, largura e altura, 4 bytes cada.
    assert_eq!(
        o["off_v4l2_cropcap_bounds"].as_u64().unwrap() + 8,
        cam::REC_LARGURA as u64
    );
    assert_eq!(
        o["off_v4l2_cropcap_bounds"].as_u64().unwrap() + 12,
        cam::REC_ALTURA as u64
    );
    // O tamanho de cada estrutura é o que o número do pedido carrega.
    for (chave, pedido) in [
        ("sizeof_v4l2_capability", cam::VIDIOC_QUERYCAP),
        ("sizeof_v4l2_input", cam::VIDIOC_ENUMINPUT),
        ("sizeof_v4l2_cropcap", cam::VIDIOC_CROPCAP),
        ("sizeof_v4l2_control", cam::VIDIOC_S_CTRL),
        ("sizeof_v4l2_format", cam::VIDIOC_S_FMT),
        ("sizeof_v4l2_requestbuffers", cam::VIDIOC_REQBUFS),
        ("sizeof_v4l2_buffer", cam::VIDIOC_QBUF),
    ] {
        assert_eq!(
            o[chave].as_u64(),
            Some(cam::arg_de(pedido).len() as u64),
            "{chave}"
        );
    }
}
