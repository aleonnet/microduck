//! A câmera do robô, aberta como o `dji_camera` a abre, e de nenhum outro jeito.
//!
//! Dois fluxos genéricos (`v4l2-ctl --stream-mmap`, em 2026-10-30) reiniciaram o robô: pularam a escolha da entrada e os
//! três controles privados. A sequência da DJI foi lida inteira da gravação dela, no laboratório
//! (`out/2026-10-30-poc2/camera/dji_camera_v4l2_decoded.strace`, linhas 312 a 318 e 495 a 510, e
//! `dji_camera_start.strace`, linhas 1056 a 1064, para os cinco `mmap`), e este módulo a repete: são os 27 passos de
//! [`PASSOS`]. **Cada resposta do driver é conferida com a gravada antes do passo seguinte, e o fluxo só é ligado se todas
//! bateram.** Um driver que responde diferente é um driver que este código não conhece: ele diz qual passo, o que esperava
//! e o que veio ([`Erro::Divergiu`]), e para.
//!
//! Dois pontos não estão na gravação. A espera antes de tirar um quadro é um `poll` (a DJI abre sem bloqueio e nunca
//! recebeu `EAGAIN`: ela espera em algum lugar que a gravação não cobriu). E a saída: a DJI foi morta no meio do fluxo em
//! toda captura, e a câmera ficou respondendo `ENODEV` até reiniciar; aqui o fluxo é desligado e os buffers são soltos
//! sempre, por qualquer caminho de saída ([`Drop`]).
//!
//! As estruturas vão em bytes, com os deslocamentos do núcleo do robô (3.10, ARM de 32 bits). O teste
//! `os_numeros_sao_os_do_compilador_arm` confere cada número com o que o compilador ARM imprimiu
//! (`s1d/tests/fixtures/v4l2_arm32.json`). O `s1d/src/cam.rs` é só comparação: foi com ele que a sequência rodou a 30,1
//! quadros por segundo em 2026-10-31.

use std::fmt;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::camera::{NV12, S1_ALTURA, S1_LARGURA, bytes_nv12};
use crate::quadros::{Quadro, Quadros};

pub const VIDIOC_QUERYCAP: u32 = 0x8068_5600;
pub const VIDIOC_ENUMINPUT: u32 = 0xc050_561a;
pub const VIDIOC_G_INPUT: u32 = 0x8004_5626;
pub const VIDIOC_S_INPUT: u32 = 0xc004_5627;
pub const VIDIOC_CROPCAP: u32 = 0xc02c_563a;
pub const VIDIOC_S_CTRL: u32 = 0xc008_561c;
pub const VIDIOC_S_FMT: u32 = 0xc0cc_5605;
pub const VIDIOC_REQBUFS: u32 = 0xc014_5608;
pub const VIDIOC_QUERYBUF: u32 = 0xc044_5609;
pub const VIDIOC_QBUF: u32 = 0xc044_560f;
pub const VIDIOC_DQBUF: u32 = 0xc044_5611;
pub const VIDIOC_STREAMON: u32 = 0x4004_5612;
pub const VIDIOC_STREAMOFF: u32 = 0x4004_5613;

pub const CAPTURA: u32 = 1; // V4L2_BUF_TYPE_VIDEO_CAPTURE
pub const MMAP: u32 = 1; // V4L2_MEMORY_MMAP
pub const FOURCC_NV12: u32 = 0x3231_564e;
pub const CAMPO_QUALQUER: u32 = 0; // V4L2_FIELD_ANY
pub const CAMPO_ENTRELACADO: u32 = 4; // V4L2_FIELD_INTERLACED, o que o driver respondeu à DJI
pub const CORES_JPEG: u32 = 7; // V4L2_COLORSPACE_JPEG
pub const CLASSE_CAMERA: u32 = 0x009a_0000;
pub const PODE_CAPTURAR: u32 = 0x0000_0001;
pub const PODE_FLUXO: u32 = 0x0400_0000;
pub const ENTRADA_CAMERA: u32 = 2;

/// Os três controles privados que a DJI grava antes do formato, na ordem, com os valores (linhas 495 a 497 da gravação).
pub const CONTROLES: [(u32, u32); 3] = [
    (CLASSE_CAMERA + 0x191b, 0),
    (CLASSE_CAMERA + 0x1934, 30),
    (CLASSE_CAMERA + 0x1935, 0),
];
pub const DRIVER: &str = "comip";
pub const SENSOR: &str = "ov5695";
/// O sensor inteiro, que o driver respondeu ao recorte.
pub const SENSOR_INTEIRO: (u32, u32) = (2592, 1944);
/// A resposta ao formato: bytes por linha e o tamanho da imagem, que é o quadro mais 32 bytes do driver.
pub const BYTES_POR_LINHA: u32 = 1920;
pub const TAMANHO_DA_IMAGEM: u32 = 1_382_432;
pub const BUFFERS: u32 = 5;
/// O tamanho de cada `mmap` da DJI: o da imagem, arredondado para páginas.
pub const TAMANHO_DO_MAPA: u32 = 1_384_448;
/// Quantos quadros são jogados fora depois de ligar: a exposição ainda está assentando (medido em 2026-10-31).
pub const AQUECIMENTO: u32 = 90;

/// Um passo da sequência: é o que uma divergência aponta, e o que o teste compara com a gravação.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Passo {
    Consulta,
    Entrada(u32),
    EscolherEntrada,
    Recorte,
    LerEntrada,
    Controle(u32),
    Formato,
    PedirBuffers,
    ConsultarBuffer(u32),
    Mapear(u32),
    Enfileirar(u32),
    Ligar,
}

/// A gravação, na ordem em que a DJI mandou.
pub const PASSOS: [Passo; 27] = [
    Passo::Consulta,
    Passo::Entrada(0),
    Passo::Entrada(1),
    Passo::EscolherEntrada,
    Passo::Recorte,
    Passo::LerEntrada,
    Passo::Controle(CONTROLES[0].0),
    Passo::Controle(CONTROLES[1].0),
    Passo::Controle(CONTROLES[2].0),
    Passo::Formato,
    Passo::PedirBuffers,
    Passo::ConsultarBuffer(0),
    Passo::Mapear(0),
    Passo::ConsultarBuffer(1),
    Passo::Mapear(1),
    Passo::ConsultarBuffer(2),
    Passo::Mapear(2),
    Passo::ConsultarBuffer(3),
    Passo::Mapear(3),
    Passo::ConsultarBuffer(4),
    Passo::Mapear(4),
    Passo::Enfileirar(0),
    Passo::Enfileirar(1),
    Passo::Enfileirar(2),
    Passo::Enfileirar(3),
    Passo::Enfileirar(4),
    Passo::Ligar,
];

/// Um buffer do driver mapeado. O de verdade é solto no `munmap`; o do teste é memória do processo.
pub struct Mapa {
    ponteiro: *mut u8,
    tamanho: usize,
    _proprio: Option<Vec<u8>>,
    ao_soltar: Option<Box<dyn FnMut() + Send>>,
}

// O ponteiro é de um mapa só deste dono, lido só por ele.
unsafe impl Send for Mapa {}

impl Mapa {
    /// Um mapa sobre memória do processo, preenchida com `byte`; `ao_soltar` roda quando ele é solto.
    pub fn proprio(tamanho: usize, byte: u8, ao_soltar: Box<dyn FnMut() + Send>) -> Self {
        let mut v = vec![byte; tamanho];
        Self {
            ponteiro: v.as_mut_ptr(),
            tamanho,
            _proprio: Some(v),
            ao_soltar: Some(ao_soltar),
        }
    }

    /// # Safety
    /// `ponteiro` tem de ser um `mmap` vivo de `tamanho` bytes; `ao_soltar` é quem o desfaz.
    pub unsafe fn do_nucleo(
        ponteiro: *mut u8,
        tamanho: usize,
        ao_soltar: Box<dyn FnMut() + Send>,
    ) -> Self {
        Self {
            ponteiro,
            tamanho,
            _proprio: None,
            ao_soltar: Some(ao_soltar),
        }
    }

    pub fn bytes(&self) -> &[u8] {
        // SAFETY: o ponteiro e o tamanho são do vetor próprio ou de um mapa vivo (contrato dos construtores).
        unsafe { std::slice::from_raw_parts(self.ponteiro, self.tamanho) }
    }
}

impl Drop for Mapa {
    fn drop(&mut self) {
        if let Some(mut f) = self.ao_soltar.take() {
            f();
        }
    }
}

/// O dispositivo: o de verdade no robô ([`linux::Video0`]) e o de mentira no teste, que responde como a gravação.
pub trait Dispositivo {
    /// `ioctl(fd, pedido, arg)`: `arg` são os bytes da estrutura, lidos e escritos no lugar.
    fn ioctl(&mut self, pedido: u32, arg: &mut [u8]) -> io::Result<()>;
    /// O `mmap` de um buffer: `tamanho` bytes em `deslocamento`, partilhado, de leitura e escrita.
    fn mapear(&mut self, tamanho: usize, deslocamento: u32) -> io::Result<Mapa>;
    /// Espera um quadro pronto por até `ms`; `false` é o prazo vencido.
    fn esperar(&mut self, ms: i32) -> io::Result<bool>;
}

#[derive(Debug)]
pub enum Erro {
    /// O driver respondeu diferente da gravação: o passo, o esperado e o que veio.
    Divergiu {
        passo: Passo,
        esperado: String,
        veio: String,
    },
    Sistema {
        passo: Passo,
        erro: io::Error,
    },
}

impl fmt::Display for Erro {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Erro::Divergiu {
                passo,
                esperado,
                veio,
            } => write!(
                f,
                "a câmera respondeu diferente da gravação da DJI em {passo:?}: esperava {esperado}, veio {veio}; o fluxo não foi ligado"
            ),
            Erro::Sistema { passo, erro } => write!(f, "a câmera recusou {passo:?}: {erro}"),
        }
    }
}

impl std::error::Error for Erro {}

fn le(b: &[u8], em: usize) -> u32 {
    u32::from_le_bytes([b[em], b[em + 1], b[em + 2], b[em + 3]])
}

fn poe(b: &mut [u8], em: usize, v: u32) {
    b[em..em + 4].copy_from_slice(&v.to_le_bytes());
}

fn texto(b: &[u8]) -> String {
    let fim = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..fim]).into_owned()
}

/// Os bytes de um pedido, zerados, no tamanho que o número do pedido carrega.
pub fn arg_de(pedido: u32) -> Vec<u8> {
    vec![0u8; ((pedido >> 16) & 0x3fff) as usize]
}

// Os deslocamentos das estruturas, no ARM de 32 bits.
pub const CAP_DRIVER: usize = 0;
pub const CAP_CAPACIDADES: usize = 84;
pub const ENT_INDICE: usize = 0;
pub const ENT_NOME: usize = 4;
pub const ENT_TIPO: usize = 36;
pub const REC_LARGURA: usize = 12;
pub const REC_ALTURA: usize = 16;
pub const CTL_ID: usize = 0;
pub const CTL_VALOR: usize = 4;
pub const FMT_TIPO: usize = 0;
pub const FMT_LARGURA: usize = 4;
pub const FMT_ALTURA: usize = 8;
pub const FMT_FOURCC: usize = 12;
pub const FMT_CAMPO: usize = 16;
pub const FMT_BYTES_POR_LINHA: usize = 20;
pub const FMT_TAMANHO: usize = 24;
pub const FMT_CORES: usize = 28;
pub const PED_QUANTOS: usize = 0;
pub const PED_TIPO: usize = 4;
pub const PED_MEMORIA: usize = 8;
pub const BUF_INDICE: usize = 0;
pub const BUF_TIPO: usize = 4;
pub const BUF_USADOS: usize = 8;
pub const BUF_SEQUENCIA: usize = 44;
pub const BUF_MEMORIA: usize = 48;
pub const BUF_DESLOCAMENTO: usize = 52;
pub const BUF_TAMANHO: usize = 56;

fn buffer(indice: u32) -> Vec<u8> {
    let mut b = arg_de(VIDIOC_QBUF);
    poe(&mut b, BUF_INDICE, indice);
    poe(&mut b, BUF_TIPO, CAPTURA);
    poe(&mut b, BUF_MEMORIA, MMAP);
    b
}

/// A câmera, com o fluxo ligado depois de a gravação inteira ter batido.
pub struct CameraS1<D: Dispositivo> {
    dev: D,
    mapas: Vec<Mapa>,
    ligado: bool,
    /// O número que o driver deu ao último quadro tirado da fila: um salto nele é quadro perdido.
    sequencia: u32,
}

/// Quantos quadros a câmera entregou e quantos o driver pulou, desde que o serviço subiu: é o que `s1.media.status` mostra.
#[derive(Default)]
pub struct Contagem {
    pub quadros: AtomicU64,
    pub perdidos: AtomicU64,
}

impl<D: Dispositivo> CameraS1<D> {
    /// Repete a gravação em `dev`. Com `Ok`, o fluxo está ligado; com `Err`, nada depois do passo que falhou foi mandado.
    pub fn abrir(dev: D) -> Result<Self, Erro> {
        // Montada já aqui: qualquer saída abaixo solta os mapas feitos até ali.
        let mut c = Self {
            dev,
            mapas: Vec::with_capacity(BUFFERS as usize),
            ligado: false,
            sequencia: 0,
        };
        c.sequencia()?;
        Ok(c)
    }

    fn pedir(&mut self, passo: Passo, pedido: u32, arg: &mut [u8]) -> Result<(), Erro> {
        self.dev
            .ioctl(pedido, arg)
            .map_err(|erro| Erro::Sistema { passo, erro })
    }

    fn sequencia(&mut self) -> Result<(), Erro> {
        let divergiu = |passo, esperado: String, veio: String| {
            Err(Erro::Divergiu {
                passo,
                esperado,
                veio,
            })
        };

        let mut cap = arg_de(VIDIOC_QUERYCAP);
        self.pedir(Passo::Consulta, VIDIOC_QUERYCAP, &mut cap)?;
        let (driver, pode) = (
            texto(&cap[CAP_DRIVER..CAP_DRIVER + 16]),
            le(&cap, CAP_CAPACIDADES),
        );
        if driver != DRIVER || pode & (PODE_CAPTURAR | PODE_FLUXO) != PODE_CAPTURAR | PODE_FLUXO {
            return divergiu(
                Passo::Consulta,
                format!("o driver {DRIVER}, com captura e fluxo"),
                format!("o driver {driver}, capacidades 0x{pode:08x}"),
            );
        }

        let mut ent = arg_de(VIDIOC_ENUMINPUT);
        self.pedir(Passo::Entrada(0), VIDIOC_ENUMINPUT, &mut ent)?;
        let (nome, tipo) = (texto(&ent[ENT_NOME..ENT_NOME + 32]), le(&ent, ENT_TIPO));
        if nome != SENSOR || tipo != ENTRADA_CAMERA {
            return divergiu(
                Passo::Entrada(0),
                format!("{SENSOR}, uma câmera"),
                format!("{nome}, tipo {tipo}"),
            );
        }
        // A DJI pergunta pela segunda entrada, e o driver diz que não há.
        let mut ent = arg_de(VIDIOC_ENUMINPUT);
        poe(&mut ent, ENT_INDICE, 1);
        if self.dev.ioctl(VIDIOC_ENUMINPUT, &mut ent).is_ok() {
            return divergiu(
                Passo::Entrada(1),
                "nenhuma segunda entrada (EINVAL)".to_owned(),
                format!(
                    "uma segunda entrada, {}",
                    texto(&ent[ENT_NOME..ENT_NOME + 32])
                ),
            );
        }

        let mut escolha = arg_de(VIDIOC_S_INPUT);
        self.pedir(Passo::EscolherEntrada, VIDIOC_S_INPUT, &mut escolha)?;

        // A DJI passa o tipo 0, que não é um tipo de buffer; repetido como está.
        let mut rec = arg_de(VIDIOC_CROPCAP);
        self.pedir(Passo::Recorte, VIDIOC_CROPCAP, &mut rec)?;
        let limites = (le(&rec, REC_LARGURA), le(&rec, REC_ALTURA));
        if limites != SENSOR_INTEIRO {
            return divergiu(
                Passo::Recorte,
                format!("limites {}x{}", SENSOR_INTEIRO.0, SENSOR_INTEIRO.1),
                format!("limites {}x{}", limites.0, limites.1),
            );
        }

        // Depois de escolher a entrada 0, o driver respondeu 1 à DJI: gravado, e não explicado.
        let mut lida = arg_de(VIDIOC_G_INPUT);
        self.pedir(Passo::LerEntrada, VIDIOC_G_INPUT, &mut lida)?;
        if le(&lida, 0) != 1 {
            return divergiu(Passo::LerEntrada, "1".to_owned(), le(&lida, 0).to_string());
        }

        for (id, valor) in CONTROLES {
            let mut ctl = arg_de(VIDIOC_S_CTRL);
            poe(&mut ctl, CTL_ID, id);
            poe(&mut ctl, CTL_VALOR, valor);
            self.pedir(Passo::Controle(id), VIDIOC_S_CTRL, &mut ctl)?;
            if le(&ctl, CTL_VALOR) != valor {
                return divergiu(
                    Passo::Controle(id),
                    valor.to_string(),
                    le(&ctl, CTL_VALOR).to_string(),
                );
            }
        }

        let mut fmt = arg_de(VIDIOC_S_FMT);
        poe(&mut fmt, FMT_TIPO, CAPTURA);
        poe(&mut fmt, FMT_LARGURA, S1_LARGURA);
        poe(&mut fmt, FMT_ALTURA, S1_ALTURA);
        poe(&mut fmt, FMT_FOURCC, FOURCC_NV12);
        poe(&mut fmt, FMT_CAMPO, CAMPO_QUALQUER);
        poe(&mut fmt, FMT_CORES, CORES_JPEG);
        self.pedir(Passo::Formato, VIDIOC_S_FMT, &mut fmt)?;
        let veio = [
            le(&fmt, FMT_LARGURA),
            le(&fmt, FMT_ALTURA),
            le(&fmt, FMT_FOURCC),
            le(&fmt, FMT_CAMPO),
            le(&fmt, FMT_BYTES_POR_LINHA),
            le(&fmt, FMT_TAMANHO),
        ];
        let esperado = [
            S1_LARGURA,
            S1_ALTURA,
            FOURCC_NV12,
            CAMPO_ENTRELACADO,
            BYTES_POR_LINHA,
            TAMANHO_DA_IMAGEM,
        ];
        if veio != esperado {
            let dizer = |v: [u32; 6]| {
                format!(
                    "{}x{}, formato 0x{:08x}, campo {}, {} bytes por linha, imagem de {} bytes",
                    v[0], v[1], v[2], v[3], v[4], v[5]
                )
            };
            return divergiu(Passo::Formato, dizer(esperado), dizer(veio));
        }

        let mut ped = arg_de(VIDIOC_REQBUFS);
        poe(&mut ped, PED_QUANTOS, BUFFERS);
        poe(&mut ped, PED_TIPO, CAPTURA);
        poe(&mut ped, PED_MEMORIA, MMAP);
        self.pedir(Passo::PedirBuffers, VIDIOC_REQBUFS, &mut ped)?;
        if le(&ped, PED_QUANTOS) != BUFFERS {
            return divergiu(
                Passo::PedirBuffers,
                BUFFERS.to_string(),
                le(&ped, PED_QUANTOS).to_string(),
            );
        }

        for i in 0..BUFFERS {
            let mut b = buffer(i);
            self.pedir(Passo::ConsultarBuffer(i), VIDIOC_QUERYBUF, &mut b)?;
            let (tamanho, onde) = (le(&b, BUF_TAMANHO), le(&b, BUF_DESLOCAMENTO));
            if tamanho != TAMANHO_DO_MAPA || onde != i * TAMANHO_DO_MAPA {
                return divergiu(
                    Passo::ConsultarBuffer(i),
                    format!("{TAMANHO_DO_MAPA} bytes em {}", i * TAMANHO_DO_MAPA),
                    format!("{tamanho} bytes em {onde}"),
                );
            }
            let mapa = self
                .dev
                .mapear(tamanho as usize, onde)
                .map_err(|erro| Erro::Sistema {
                    passo: Passo::Mapear(i),
                    erro,
                })?;
            self.mapas.push(mapa);
        }
        for i in 0..BUFFERS {
            self.pedir(Passo::Enfileirar(i), VIDIOC_QBUF, &mut buffer(i))?;
        }
        let mut tipo = arg_de(VIDIOC_STREAMON);
        poe(&mut tipo, 0, CAPTURA);
        self.pedir(Passo::Ligar, VIDIOC_STREAMON, &mut tipo)?;
        self.ligado = true;
        Ok(())
    }

    /// Espera um quadro por até `ms` e o tira da fila: o índice do buffer, para [`Self::devolver`], e os bytes que o
    /// driver preencheu. `None` é o prazo vencido.
    pub fn quadro(&mut self, ms: i32) -> io::Result<Option<(u32, &[u8])>> {
        if !self.dev.esperar(ms)? {
            return Ok(None);
        }
        let mut b = arg_de(VIDIOC_DQBUF);
        poe(&mut b, BUF_TIPO, CAPTURA);
        poe(&mut b, BUF_MEMORIA, MMAP);
        self.dev.ioctl(VIDIOC_DQBUF, &mut b)?;
        let indice = le(&b, BUF_INDICE);
        self.sequencia = le(&b, BUF_SEQUENCIA);
        let mapa = self.mapas.get(indice as usize).ok_or_else(|| {
            io::Error::other(format!(
                "o driver devolveu o buffer {indice}, e são {BUFFERS}"
            ))
        })?;
        let usados = (le(&b, BUF_USADOS) as usize).min(mapa.bytes().len());
        Ok(Some((indice, &mapa.bytes()[..usados])))
    }

    /// O número que o driver deu ao último quadro tirado da fila.
    pub fn sequencia_do_ultimo(&self) -> u32 {
        self.sequencia
    }

    /// Devolve ao driver um buffer tirado da fila.
    pub fn devolver(&mut self, indice: u32) -> io::Result<()> {
        self.dev.ioctl(VIDIOC_QBUF, &mut buffer(indice))
    }
}

impl<D: Dispositivo> Drop for CameraS1<D> {
    /// Por qualquer caminho de saída: o fluxo é desligado, e só depois os buffers são soltos.
    fn drop(&mut self) {
        if self.ligado {
            let mut tipo = arg_de(VIDIOC_STREAMOFF);
            poe(&mut tipo, 0, CAPTURA);
            if let Err(e) = self.dev.ioctl(VIDIOC_STREAMOFF, &mut tipo) {
                tracing::error!(erro = %e, "a câmera não desligou o fluxo");
            }
            self.ligado = false;
        }
        self.mapas.clear();
    }
}

/// Quanto a câmera espera um quadro antes de dizer que ele não veio: a 30 por segundo, é o tempo de sessenta.
const PRAZO_DO_QUADRO_MS: i32 = 2000;
/// Quantos prazos seguidos sem quadro fazem a câmera desistir.
const PRAZOS_SEM_QUADRO: u32 = 3;

/// A câmera do robô, até `parar` ficar verdadeiro. Todo quadro é tirado da fila e devolvido, para o driver seguir; ele só
/// é copiado quando alguém o pediu, sem os 32 bytes que o driver anexa.
///
/// A câmera é aberta uma vez. Se o driver diverge da gravação, ou a captura falha, o motivo vai a `dizer` (é o que
/// `s1.media.status` mostra) e a câmera fica parada até o serviço subir de novo: tentar outra vez num driver que não se
/// conhece é o que reiniciou o robô em 2026-10-30.
pub fn rodar<D: Dispositivo>(
    quadros: Quadros,
    dev: D,
    aquecimento: u32,
    dizer: impl Fn(Option<String>),
    parar: Arc<AtomicBool>,
) {
    rodar_contando(
        quadros,
        dev,
        aquecimento,
        dizer,
        &Contagem::default(),
        parar,
    );
}

/// [`rodar`], contando em `contagem` cada quadro que a câmera entrega e cada um que o driver pulou.
pub fn rodar_contando<D: Dispositivo>(
    quadros: Quadros,
    dev: D,
    aquecimento: u32,
    dizer: impl Fn(Option<String>),
    contagem: &Contagem,
    parar: Arc<AtomicBool>,
) {
    let mut anterior: Option<u32> = None;
    let mut camera = match CameraS1::abrir(dev) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(erro = %e, "a câmera do robô não abriu");
            dizer(Some(e.to_string()));
            return;
        }
    };
    let quadro_inteiro = bytes_nv12(S1_LARGURA, S1_ALTURA);
    let (mut vistos, mut vazios) = (0u32, 0u32);
    while !parar.load(Ordering::Relaxed) {
        let (indice, copia) = match camera.quadro(PRAZO_DO_QUADRO_MS) {
            Ok(Some((indice, bytes))) => {
                vazios = 0;
                contagem.quadros.fetch_add(1, Ordering::Relaxed);
                if vistos < aquecimento {
                    vistos += 1;
                    if vistos == aquecimento {
                        tracing::info!(quadros = aquecimento, "a câmera do robô aqueceu");
                        dizer(None);
                    }
                    (indice, None)
                } else if bytes.len() < quadro_inteiro {
                    let motivo = format!(
                        "a câmera entregou {} bytes, e o quadro tem {quadro_inteiro}",
                        bytes.len()
                    );
                    tracing::error!(motivo, "a câmera do robô parou");
                    dizer(Some(motivo));
                    return;
                } else if quadros.levar_pedido() {
                    (indice, Some(bytes[..quadro_inteiro].to_vec()))
                } else {
                    (indice, None)
                }
            }
            Ok(None) => {
                vazios += 1;
                if vazios >= PRAZOS_SEM_QUADRO {
                    let motivo = format!(
                        "a câmera ficou {} s sem entregar quadro",
                        i64::from(PRAZO_DO_QUADRO_MS) * i64::from(PRAZOS_SEM_QUADRO) / 1000
                    );
                    tracing::error!(motivo, "a câmera do robô parou");
                    dizer(Some(motivo));
                    return;
                }
                continue;
            }
            Err(e) => {
                tracing::error!(erro = %e, "a câmera do robô parou");
                dizer(Some(format!("a captura falhou: {e}")));
                return;
            }
        };
        // O driver numera os quadros: um salto no número é quadro que ele não entregou.
        let numero = camera.sequencia_do_ultimo();
        if let Some(a) = anterior
            && numero > a.wrapping_add(1)
        {
            contagem
                .perdidos
                .fetch_add(u64::from(numero - a - 1), Ordering::Relaxed);
        }
        anterior = Some(numero);
        if let Some(dados) = copia {
            quadros.entregar(Quadro {
                largura: S1_LARGURA,
                altura: S1_ALTURA,
                formato: NV12,
                capturado_em: std::time::SystemTime::now(),
                dados,
            });
        }
        if let Err(e) = camera.devolver(indice) {
            tracing::error!(erro = %e, "a câmera do robô parou");
            dizer(Some(format!("a câmera não aceitou o buffer de volta: {e}")));
            return;
        }
    }
}

#[cfg(target_os = "linux")]
pub mod linux {
    //! O dispositivo de verdade: `open`, `ioctl`, `mmap` e `poll`, como a DJI abre (`O_RDWR | O_NONBLOCK`).
    use super::{Dispositivo, Mapa};
    use std::ffi::CString;
    use std::io;
    use std::os::unix::io::RawFd;

    pub struct Video0 {
        fd: RawFd,
    }

    impl Video0 {
        pub fn abrir(caminho: &str) -> io::Result<Self> {
            let c = CString::new(caminho).map_err(|_| io::Error::other("caminho com NUL"))?;
            // SAFETY: um texto C válido; as opções são as da DJI.
            let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK) };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { fd })
        }
    }

    impl Drop for Video0 {
        fn drop(&mut self) {
            // SAFETY: o nosso descritor, fechado uma vez.
            unsafe {
                libc::close(self.fd);
            }
        }
    }

    impl Dispositivo for Video0 {
        fn ioctl(&mut self, pedido: u32, arg: &mut [u8]) -> io::Result<()> {
            // SAFETY: `arg` tem o tamanho que o número do pedido carrega (`arg_de`), e é gravável inteiro.
            let rc = unsafe {
                libc::ioctl(
                    self.fd,
                    pedido as libc::c_ulong as _,
                    arg.as_mut_ptr().cast::<libc::c_void>(),
                )
            };
            if rc < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }

        fn mapear(&mut self, tamanho: usize, deslocamento: u32) -> io::Result<Mapa> {
            // SAFETY: o mapa partilhado de um buffer do driver, no deslocamento que a consulta deu.
            let p = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    tamanho,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    self.fd,
                    // `off_t` tem 32 bits no robô; os cinco deslocamentos ficam abaixo de 7 MB.
                    deslocamento as libc::off_t,
                )
            };
            if p == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            let endereco = p as usize;
            let soltar = Box::new(move || {
                // SAFETY: o mapa feito acima, deste tamanho, solto uma vez.
                unsafe {
                    libc::munmap(endereco as *mut libc::c_void, tamanho);
                }
            });
            // SAFETY: um mapa novo de `tamanho` bytes, que só `soltar` desfaz.
            Ok(unsafe { Mapa::do_nucleo(p.cast::<u8>(), tamanho, soltar) })
        }

        fn esperar(&mut self, ms: i32) -> io::Result<bool> {
            let mut pfd = libc::pollfd {
                fd: self.fd,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: um `pollfd`, nosso.
            let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
            if rc < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(rc > 0 && pfd.revents & libc::POLLIN != 0)
        }
    }
}
