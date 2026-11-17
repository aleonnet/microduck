//! O que o S1 e a estação Hailo falam: o anúncio da estação na rede e o cabeçalho de cada mensagem.
//!
//! O formato mora em `tools/hailo/estacao_anuncio.py`, no laboratório, com a descrição inteira. Aqui é a leitura do lado
//! do S1, conferida contra um pacote e uma mensagem gravados por lá (`tests/estacao.rs`).
//!
//! O S1 não lê o contrato das funções: lê o anúncio, que diz que entradas mandar, em que ritmo, e o que cada função
//! ligada entrega.

use std::net::IpAddr;

use serde::Deserialize;

/// A versão do protocolo: a do campo `rms1_hailo` do anúncio e do primeiro byte de cada cabeçalho.
pub const VERSAO: u8 = 2;
/// Sem anúncio por este tempo, a estação saiu: seis anúncios perdidos, como para o robô.
pub const ANUNCIO_VALE: std::time::Duration = std::time::Duration::from_secs(3);
/// A maior carga aceita: o quadro cheio tem 1.382.400 bytes.
pub const CARGA_MAXIMA: u32 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Anuncio {
    /// A porta TCP dos quadros.
    pub quadros: u16,
    /// A porta HTTP do painel.
    pub painel: u16,
    pub entradas: Vec<Entrada>,
    pub funcoes: Vec<Funcao>,
    /// A estação sabe marcar cada quadro atendido ([`Tipo::Atendido`]). Uma estação de antes disto não diz nada.
    #[serde(default)]
    pub atendido: bool,
    /// A estação sabe mandar o resultado de um pedido e a fala ([`Tipo::Resultado`], [`Tipo::Fala`]), a quem disser o que
    /// recebe ([`Tipo::Sabe`]). Uma estação de antes disto não diz nada, e fecharia a conexão ao receber o aviso.
    #[serde(default)]
    pub sabe: bool,
}

/// Uma entrada que a estação quer receber.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Entrada {
    /// O canal das mensagens desta entrada.
    pub n: u16,
    pub id: String,
    pub formato: String,
    /// Quantas por segundo: o maior ritmo entre as funções ligadas que a usam.
    pub ritmo: Option<f64>,
    /// Em que formato a estação prefere o quadro: `cru`, `h264` ou `jpeg` (os dois últimos feitos pelo chip do S1). Uma
    /// estação de antes disto não diz nada, e recebe o cru.
    pub prefere: Option<String>,
    /// A qualidade do formato preferido: a taxa do H.264 em Mb/s (0 = o máximo do chip), ou o nível do JPEG (0 a 10).
    pub qualidade: Option<f64>,
}

/// Uma função ligada na estação.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Funcao {
    /// O canal das entregas desta função.
    pub n: u16,
    pub id: String,
    /// O tipo de dado da entrega.
    pub entrega: String,
    /// O leitor que produziu a entrega: diz o formato da carga.
    pub leitor: Option<String>,
    /// A notificação que o S1 publica com a entrega. Sem ela, a entrega só é contada.
    pub publica: Option<String>,
    pub nota: Option<f64>,
    pub iou: Option<f64>,
}

/// Se um anúncio vindo deste endereço vale: rede privada (10/8, 172.16/12, 192.168/16) ou a própria máquina (127/8), que
/// entra porque o serviço da estação também roda no Mac, com o Hailo de mentira.
pub fn remetente_vale(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback(),
        IpAddr::V6(_) => false,
    }
}

/// O anúncio lido, ou `None`: de endereço que não vale, que não é JSON, de outra versão, ou malformado.
pub fn ler_anuncio(pacote: &[u8], remetente: IpAddr) -> Option<Anuncio> {
    if !remetente_vale(remetente) {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(pacote).ok()?;
    if v.get("rms1_hailo")?.as_u64()? != u64::from(VERSAO) {
        return None;
    }
    let a: Anuncio = serde_json::from_value(v).ok()?;
    let canais_validos = a.entradas.iter().all(|e| e.n > 0) && a.funcoes.iter().all(|f| f.n > 0);
    (a.quadros > 0 && a.painel > 0 && canais_validos).then_some(a)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tipo {
    /// Do S1 para a estação: o canal é o número da entrada.
    Entrada = 1,
    /// Da estação para o S1: o canal é o número da função.
    Entrega = 2,
    /// Da estação para o S1: o canal da mensagem recusada, e o motivo em texto.
    Erro = 3,
    /// Do S1 para a estação, uma vez por segundo: a ida e volta e os quadros por segundo, em JSON.
    Medida = 7,
    /// Do S1 para a estação: como [`Tipo::Entrada`], com o quadro em H.264, feito pelo chip do S1. Um quadro por mensagem.
    EntradaH264 = 9,
    /// Do S1 para a estação: idem, em JPEG.
    EntradaJpeg = 10,
    /// O quadro atendido. Do S1 para a estação, uma vez ao ligar, sem carga: o pedido para ela marcar cada quadro. Da estação
    /// para o S1, depois de tudo o que ela tinha a mandar sobre um quadro: o canal leva o número do quadro, contado desde o
    /// pedido, nos 16 bits de baixo. É por ele que o S1 sabe de qual quadro é cada entrega. Só entre um S1 e uma estação que
    /// se entendem: a estação diz no anúncio (`atendido`) que sabe marcar, e só marca para o S1 que pediu.
    Atendido = 11,
    /// Do S1 para a estação, uma vez ao ligar: JSON `{"recebe": [...]}`, com o que este S1 sabe receber (`resultado`,
    /// `fala`). Só para a estação que diz no anúncio que sabe (`sabe`).
    Sabe = 19,
    /// Da estação para o S1: um acontecimento de um pedido a uma função sob demanda, em JSON (o começo, cada etapa, cada
    /// pedaço da resposta, o fim com a entrega). O canal é o número da função.
    Resultado = 20,
    /// Da estação para o S1: um pedaço do som da fala, PCM de 16 bits, um canal; o canal leva a taxa, em centenas de
    /// amostras por segundo. Sem carga: a fala acabou, ou foi interrompida.
    Fala = 21,
}

/// Os 8 bytes antes de cada carga: versão, tipo, canal e tamanho, little-endian.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cabecalho {
    pub tipo: Tipo,
    pub canal: u16,
    pub tamanho: u32,
}

impl Cabecalho {
    pub const BYTES: usize = 8;

    pub fn bytes(self) -> [u8; Self::BYTES] {
        let mut b = [0u8; Self::BYTES];
        b[0] = VERSAO;
        b[1] = self.tipo as u8;
        b[2..4].copy_from_slice(&self.canal.to_le_bytes());
        b[4..8].copy_from_slice(&self.tamanho.to_le_bytes());
        b
    }

    /// O cabeçalho lido, ou o motivo da recusa.
    pub fn ler(b: &[u8; Self::BYTES]) -> Result<Self, String> {
        if b[0] != VERSAO {
            return Err(format!(
                "versão {} do protocolo, e este S1 fala a {VERSAO}",
                b[0]
            ));
        }
        // Os tipos 4 a 6, o 8 e de 12 a 18 só existem dentro da estação: chegando ao S1, são erro de quem mandou.
        let tipo = match b[1] {
            1 => Tipo::Entrada,
            2 => Tipo::Entrega,
            3 => Tipo::Erro,
            7 => Tipo::Medida,
            9 => Tipo::EntradaH264,
            10 => Tipo::EntradaJpeg,
            11 => Tipo::Atendido,
            19 => Tipo::Sabe,
            20 => Tipo::Resultado,
            21 => Tipo::Fala,
            t => return Err(format!("tipo de mensagem desconhecido: {t}")),
        };
        let tamanho = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
        if tamanho > CARGA_MAXIMA {
            return Err(format!(
                "carga de {tamanho} bytes, acima do máximo de {CARGA_MAXIMA}"
            ));
        }
        Ok(Self {
            tipo,
            canal: u16::from_le_bytes([b[2], b[3]]),
            tamanho,
        })
    }
}
