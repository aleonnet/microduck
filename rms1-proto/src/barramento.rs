//! A telemetria do corpo, em quadros: como assinar e como ler o que chega.
//!
//! O controlador tem um barramento virtual: quem assina recebe os dados colados num pacote, sem separador, na ordem em
//! que pediu (`biblioteca/guia/telemetria.md`). A assinatura aqui é a da DJI, byte a byte, com a ordem medida nas
//! gravações; `tests/barramento.rs` confere contra quadros lidos da UART do robô.
//!
//! [`Pedido`] é a lista inteira do que pode sair. O reset de nó não existe aqui, de propósito: ele deixou o barramento do
//! robô morto até desligar na chave.
//!
//! Sem E/S e sem convenção do contrato: os valores saem como o corpo os manda, x à frente, y à direita e z para baixo.

use crate::comando::byte_do_no;
use crate::quadro::{PedeAck, Quadro};

/// `(cmd_set, cmd_id)`.
pub mod id {
    pub const REGISTRAR: (u8, u8) = (0x48, 0x01);
    pub const ASSINAR: (u8, u8) = (0x48, 0x03);
    pub const CANCELAR: (u8, u8) = (0x48, 0x04);
    pub const PACOTE: (u8, u8) = (0x48, 0x08);
    /// `telemetria.RM_ATTITUDE_EVENT`.
    pub const ESTADO_CHASSI: (u8, u8) = (0x3f, 0x29);
    /// `armadura.RM_HIT_EVENT`.
    pub const GOLPE: (u8, u8) = (0x3f, 0x02);
    /// `armadura.RM_IR_EVENT`.
    pub const INFRAVERMELHO: (u8, u8) = (0x3f, 0x10);
    /// `torre`, os 9 bytes de `GIMBAL_POS_INFO`.
    pub const TORRE_POSICAO: (u8, u8) = (0x3f, 0x76);
    /// `torre.GIMBAL_PUSH_POSITION`.
    pub const TORRE_ATITUDE: (u8, u8) = (0x04, 0x05);
}

/// Quem assina: é o nó da central da DJI, que paramos.
pub const NO_ASSINANTE: u8 = byte_do_no(900);
/// Quem responde: o controlador do corpo, no barramento.
pub const CONTROLADOR: u8 = byte_do_no(300);
/// A versão do nó, como `rm_module.py:Tank.set_sub_node`.
const VERSAO_NO: u32 = 0x0300_0000;
/// Bit 0, carimbo de tempo; bit 1, parar ao desconectar. O `dji_hdvt_uav` usa 3, literal.
const CONFIG_DJI: u8 = 3;
/// Família do lado do controlador. Os clientes da central usam outra.
const FAMILIA_CONTROLADOR: u32 = 0x0002_0003;
/// A mensagem e o ritmo da assinatura da DJI.
pub const MENSAGEM: u8 = 1;
pub const FREQUENCIA_HZ: u16 = 50;
/// O byte da resposta da DJI a cada pacote.
const RESPOSTA_AO_PACOTE: u8 = 1;
/// Modo e número da mensagem, antes dos dados.
const CABECALHO: usize = 2;
/// Dois inteiros de 32 bits: milissegundos e microssegundos do relógio do robô.
const TEMPO: usize = 8;

/// Códigos de sucesso. Qualquer outro é recusa, e o sentido está em [`sentido_do_codigo`].
pub const OK: u8 = 0x00;
pub const NO_JA_EXISTE: u8 = 0x50;
pub const MENSAGEM_NAO_EXISTE: u8 = 0x0c;
pub const MENSAGEM_JA_EXISTE: u8 = 0x04;
pub const DADO_JA_ASSINADO: u8 = 0x23;

/// Um dado do barramento, na tabela do `dji_hdvt_uav`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dado {
    pub nome: &'static str,
    pub identificador: u32,
    pub tamanho: usize,
}

pub const TORRE_BASICO: Dado = Dado {
    nome: "GIMBAL_BASIC_INFO",
    identificador: 0x8829_02a7,
    tamanho: 14,
};
pub const VELOCIDADE: Dado = Dado {
    nome: "NS_VEL_INFO",
    identificador: 0x4c3e_3e66,
    tamanho: 24,
};
pub const BATERIA: Dado = Dado {
    nome: "BATTERY_INFO",
    identificador: 0xd7f5_dcfb,
    tamanho: 10,
};
pub const RODAS: Dado = Dado {
    nome: "ESC_BASIC_INFO",
    identificador: 0xe226_a309,
    tamanho: 36,
};
pub const POSICAO: Dado = Dado {
    nome: "TAG_RM_NS_POS",
    identificador: 0x47e6_f7b3,
    tamanho: 12,
};
pub const IMU: Dado = Dado {
    nome: "TAG_RM_NS_IMU_INFO",
    identificador: 0xdc1c_1df4,
    tamanho: 24,
};
pub const ESTADO: Dado = Dado {
    nome: "RM_NS_SA_STATUS",
    identificador: 0x0858_c503,
    tamanho: 4,
};
pub const ATITUDE: Dado = Dado {
    nome: "TAG_RM_ATTI_INFO",
    identificador: 0x1d13_ee42,
    tamanho: 12,
};
pub const SERVO: Dado = Dado {
    nome: "SERVO_DATA_INFO",
    identificador: 0xa7ed_801e,
    tamanho: 17,
};

/// A assinatura da DJI a 50 Hz, na ordem medida em 4 de 6 gravações.
pub const ASSINATURA: [Dado; 9] = [
    TORRE_BASICO,
    VELOCIDADE,
    BATERIA,
    RODAS,
    POSICAO,
    IMU,
    ESTADO,
    ATITUDE,
    SERVO,
];

/// Tudo o que este serviço pede ao barramento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pedido {
    RegistrarNo,
    Cancelar {
        mensagem: u8,
    },
    Assinar {
        mensagem: u8,
        dados: &'static [Dado],
        frequencia: u16,
    },
    /// Resposta a um pacote, com a sequência dele.
    Responder {
        seq: u16,
    },
}

impl Pedido {
    pub fn id(&self) -> (u8, u8) {
        match self {
            Self::RegistrarNo => id::REGISTRAR,
            Self::Cancelar { .. } => id::CANCELAR,
            Self::Assinar { .. } => id::ASSINAR,
            Self::Responder { .. } => id::PACOTE,
        }
    }

    /// `seq` vale para os três pedidos; a resposta usa a do pacote.
    pub fn quadro(&self, seq: u16) -> Quadro {
        let (seq, resposta, pede, dados) = match *self {
            Self::RegistrarNo => {
                let mut d = vec![NO_ASSINANTE];
                d.extend_from_slice(&VERSAO_NO.to_le_bytes());
                (seq, false, PedeAck::AoTerminar, d)
            }
            Self::Cancelar { mensagem } => (
                seq,
                false,
                PedeAck::AoTerminar,
                vec![0, NO_ASSINANTE, mensagem],
            ),
            Self::Assinar {
                mensagem,
                dados,
                frequencia,
            } => {
                let mut d = vec![NO_ASSINANTE, mensagem, CONFIG_DJI, 0, dados.len() as u8];
                for dado in dados {
                    d.extend_from_slice(&dado.identificador.to_le_bytes());
                    d.extend_from_slice(&FAMILIA_CONTROLADOR.to_le_bytes());
                }
                d.extend_from_slice(&frequencia.to_le_bytes());
                (seq, false, PedeAck::AoTerminar, d)
            }
            Self::Responder { seq: a } => (a, true, PedeAck::Agora, vec![RESPOSTA_AO_PACOTE]),
        };
        let (cmd_set, cmd_id) = self.id();
        Quadro {
            origem: NO_ASSINANTE,
            destino: CONTROLADOR,
            seq,
            resposta,
            pede,
            cmd_set,
            cmd_id,
            dados,
        }
    }
}

/// O sentido de um código de resposta, lido na biblioteca do barramento, em `dji_hdvt_uav`. Que o controlador rode a mesma
/// biblioteca é inferência forte, e não prova: por isso o código sempre aparece junto.
pub fn sentido_do_codigo(codigo: u8) -> &'static str {
    match codigo {
        0x00 => "sucesso",
        0x01 => "tamanho errado",
        0x02 => "versão do nó não confere",
        0x04 => "esse número de mensagem já existe",
        0x06 => "frequência inválida",
        0x07 => "pacote maior que 800 bytes",
        0x08 => "fluxo acima do limite",
        0x0c => "mensagem a cancelar não existe",
        0x0e => "nó ou versão ilegal",
        0x0f => "nó não existe",
        0x20 => "lista vazia",
        0x21 => "identificador ilegal",
        0x22 => "sem autoridade",
        0x23 => "esse dado já está assinado por esse nó",
        0x24 => "nada montado para esse dado",
        0x26 => "sem espaço para mensagem",
        0x29 => "mais de 10 dados",
        0x50 => "nó já existe",
        0x51 => "limite de 7 nós",
        _ => "código desconhecido",
    }
}

/// Onde cada dado cai, a partir da lista que foi assinada. Quem assina sabe a ordem: é como fazem o app, o Python e a torre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    torre: usize,
    velocidade: usize,
    bateria: usize,
    rodas: usize,
    posicao: usize,
    imu: usize,
    estado: usize,
    atitude: usize,
    pub total: usize,
}

impl Layout {
    /// `None` se a lista não trouxer os dados que este serviço lê.
    pub fn de(lista: &[Dado]) -> Option<Self> {
        let mut p = CABECALHO + TEMPO;
        let mut pos = [None; 8];
        for dado in lista {
            let i = [
                TORRE_BASICO,
                VELOCIDADE,
                BATERIA,
                RODAS,
                POSICAO,
                IMU,
                ESTADO,
                ATITUDE,
            ]
            .iter()
            .position(|d| d == dado);
            if let Some(i) = i {
                pos[i] = Some(p);
            }
            p += dado.tamanho;
        }
        Some(Self {
            torre: pos[0]?,
            velocidade: pos[1]?,
            bateria: pos[2]?,
            rodas: pos[3]?,
            posicao: pos[4]?,
            imu: pos[5]?,
            estado: pos[6]?,
            atitude: pos[7]?,
            total: p,
        })
    }

    pub fn da_assinatura() -> Self {
        Self::de(&ASSINATURA).expect("a assinatura traz todos os dados")
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bateria {
    pub milivolts: u16,
    /// Décimos de grau Celsius.
    pub temperatura: i16,
    /// Miliamperes, negativa ao descarregar.
    pub corrente: i32,
    pub percentual: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rodas {
    /// rpm, no sinal do fio: a roda 2 e a 3 invertidas, como `0x3f/0x20`.
    pub rotacao: [i16; 4],
    /// De 0 a 32767, para uma volta.
    pub angulo: [u16; 4],
    pub estado: [u8; 4],
}

/// Um pacote de telemetria, como o corpo o mandou: x à frente, y à direita, z para baixo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pacote {
    pub mensagem: u8,
    pub tempo_ms: u32,
    pub tempo_us: u32,
    /// Graus, arredondado. Medido: é o rumo da torre.
    pub torre_basico: f32,
    pub velocidade_solo: [f32; 3],
    pub velocidade_corpo: [f32; 3],
    pub bateria: Bateria,
    pub rodas: Rodas,
    pub posicao: [f32; 3],
    /// Em g: -1 em z com o robô parado.
    pub aceleracao: [f32; 3],
    /// Em rad/s.
    pub giro: [f32; 3],
    pub estado: u32,
    /// Rumo, inclinação e rolagem, em graus.
    pub atitude: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{comando}: {tem} bytes, esperava {precisa}")]
pub struct ErroLeitura {
    pub comando: &'static str,
    pub tem: usize,
    pub precisa: usize,
}

fn f32_em(d: &[u8], p: usize) -> f32 {
    f32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}
fn u32_em(d: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}
fn i16_em(d: &[u8], p: usize) -> i16 {
    i16::from_le_bytes([d[p], d[p + 1]])
}
fn u16_em(d: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([d[p], d[p + 1]])
}
fn f3(d: &[u8], p: usize) -> [f32; 3] {
    [f32_em(d, p), f32_em(d, p + 4), f32_em(d, p + 8)]
}

fn exigir(comando: &'static str, d: &[u8], precisa: usize) -> Result<(), ErroLeitura> {
    if d.len() < precisa {
        return Err(ErroLeitura {
            comando,
            tem: d.len(),
            precisa,
        });
    }
    Ok(())
}

impl Pacote {
    /// Tamanho errado é erro: outro tamanho é outra lista, e ler assim daria valores plausíveis e falsos.
    pub fn ler(d: &[u8], l: &Layout) -> Result<Self, ErroLeitura> {
        if d.len() != l.total {
            return Err(ErroLeitura {
                comando: "pacote de telemetria",
                tem: d.len(),
                precisa: l.total,
            });
        }
        let r = l.rodas;
        let b = l.bateria;
        Ok(Self {
            mensagem: d[1],
            tempo_ms: u32_em(d, 2),
            tempo_us: u32_em(d, 6),
            torre_basico: f32_em(d, l.torre),
            velocidade_solo: f3(d, l.velocidade),
            velocidade_corpo: f3(d, l.velocidade + 12),
            bateria: Bateria {
                milivolts: u16_em(d, b),
                temperatura: i16_em(d, b + 2),
                corrente: u32_em(d, b + 4) as i32,
                percentual: d[b + 8],
            },
            rodas: Rodas {
                rotacao: [
                    i16_em(d, r),
                    i16_em(d, r + 2),
                    i16_em(d, r + 4),
                    i16_em(d, r + 6),
                ],
                angulo: [
                    u16_em(d, r + 8),
                    u16_em(d, r + 10),
                    u16_em(d, r + 12),
                    u16_em(d, r + 14),
                ],
                estado: [d[r + 32], d[r + 33], d[r + 34], d[r + 35]],
            },
            posicao: f3(d, l.posicao),
            aceleracao: f3(d, l.imu),
            giro: f3(d, l.imu + 12),
            estado: u32_em(d, l.estado),
            atitude: f3(d, l.atitude),
        })
    }
}

/// O número da mensagem de um pacote qualquer, mesmo de quem não sabemos a ordem.
pub fn mensagem_do_pacote(d: &[u8]) -> Option<u8> {
    d.get(1).copied()
}

/// `0x3f/0x29`: os mesmos bits do estado do pacote.
pub fn ler_estado_chassi(d: &[u8]) -> Result<u32, ErroLeitura> {
    exigir("estado do chassi", d, 4)?;
    Ok(u32_em(d, 0))
}

/// `0x3f/0x02`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Golpe {
    /// 1 traseira, 2 dianteira, 3 esquerda, 4 direita, 5 topo esquerda, 6 topo direita (`rm_ctrl.py:hit_event_process`).
    pub placa: u8,
    pub tipo: u8,
    pub microfone: u16,
    pub aceleracao: u16,
}

pub fn ler_golpe(d: &[u8]) -> Result<Golpe, ErroLeitura> {
    exigir("golpe", d, 5)?;
    Ok(Golpe {
        placa: d[0] >> 4,
        tipo: d[0] & 0x0f,
        microfone: u16_em(d, 1),
        aceleracao: u16_em(d, 3),
    })
}

/// `0x3f/0x10`: bit 0, topo esquerda; bit 1, topo direita (`rm_ctrl.py:2399`).
pub fn ler_infravermelho(d: &[u8]) -> Result<u8, ErroLeitura> {
    exigir("infravermelho", d, 3)?;
    Ok(d[1] & 0x0f)
}

/// `0x3f/0x76`, em décimos de grau. Medido nas gravações: o rumo em 4 é em relação ao chassi, e o em 0, no solo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TorrePosicao {
    pub rumo_solo: i16,
    pub inclinacao_solo: i16,
    pub rumo: i16,
    pub inclinacao: i16,
}

pub fn ler_torre_posicao(d: &[u8]) -> Result<TorrePosicao, ErroLeitura> {
    exigir("posição da torre", d, 9)?;
    Ok(TorrePosicao {
        rumo_solo: i16_em(d, 0),
        inclinacao_solo: i16_em(d, 2),
        rumo: i16_em(d, 4),
        inclinacao: i16_em(d, 6),
    })
}

/// O código de uma resposta ao registro, à assinatura ou ao cancelamento.
pub fn ler_codigo(d: &[u8]) -> Result<u8, ErroLeitura> {
    exigir("resposta do barramento", d, 1)?;
    Ok(d[0])
}
