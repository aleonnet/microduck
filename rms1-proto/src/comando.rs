//! Os comandos que `rms1d` manda ao corpo nesta fase. Cada um cita a mensagem da biblioteca
//! (`biblioteca/guia/chassi.md` e `torre.md`) e a linha da fonte da DJI.
//!
//! O reset de nó de telemetria não existe aqui, de propósito: ele deixou o barramento do robô morto até
//! desligar na chave. `tests/comandos.rs` varre os comandos atrás dele.

use std::collections::HashMap;

use crate::quadro::{PedeAck, Quadro};

/// `(cmd_set, cmd_id)`.
pub mod id {
    /// `chassi.RM_FC_RMC`: a alavanca neutra é a permissão do chassi para andar.
    pub const PERMISSAO: (u8, u8) = (0x3f, 0x60);
    /// `chassi.RM_WORK_MODE_SET`.
    pub const MODO_CHASSI: (u8, u8) = (0x3f, 0x19);
    /// `chassi.RM_WHEEL_SPEED_SET`.
    pub const RODAS: (u8, u8) = (0x3f, 0x20);
    /// `chassi.RM_FOLLOW_MODE_SET`.
    pub const SEGUIR: (u8, u8) = (0x3f, 0x22);
    /// `chassi.RM_SPEED_MODE_SET`.
    pub const SOBREPOR: (u8, u8) = (0x3f, 0x28);
    /// `torre.RM_GIMBAL_DEGREE_SET`.
    pub const TORRE_ANGULOS: (u8, u8) = (0x3f, 0xb0);
    /// `torre.GIMBAL_SET_MODE`.
    pub const MODO_TORRE: (u8, u8) = (0x04, 0x4c);
}

/// Quem é quem no barramento: o número da DJI, centena o hospedeiro e resto o índice (`306` é o
/// hospedeiro 3 no índice 6, ou seja `0xC3`). Vêm de `rms1d.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nos {
    pub remetente: u8,
    pub remetente_alavanca: u8,
    pub chassi: u8,
    pub torre: u8,
}

/// Número da DJI para o byte do quadro: `índice * 32 + hospedeiro` (`tools.hostid2packid`).
pub const fn byte_do_no(numero: u16) -> u8 {
    let hospedeiro = numero / 100;
    let indice = numero % 100;
    (indice * 32 + hospedeiro) as u8
}

impl Default for Nos {
    fn default() -> Self {
        Self {
            remetente: byte_do_no(906),
            remetente_alavanca: byte_do_no(900),
            chassi: byte_do_no(306),
            torre: byte_do_no(400),
        }
    }
}

/// Qual quadro dá a permissão de andar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PermissaoForma {
    /// O que a DJI envia: 900 para 306, gravado em 6784 quadros da `bancada-150s`.
    #[default]
    Dji,
    /// Plano B: os bytes do proroklab ao nó 3, que moveram o robô em 2026-10-29.
    Proroklab,
}

/// A alavanca neutra como a DJI a envia, centro 1024 em todos os campos (`heartbeat_real.bin`).
const ALAVANCA_NEUTRA_DJI: [u8; 14] = [
    0x00, 0x04, 0x20, 0x00, 0x01, 0x08, 0x40, 0x00, 0x02, 0x10, 0x04, 0x00, 0x00, 0x04,
];
/// Os bytes do proroklab, plano B (`poc_hb.bin`).
const ALAVANCA_NEUTRA_PROROKLAB: [u8; 14] = [
    0x00, 0x04, 0x20, 0x00, 0x01, 0x00, 0x40, 0x00, 0x02, 0x10, 0x04, 0x03, 0x00, 0x04,
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Comando {
    /// `0x3f/0x60`.
    Permissao(PermissaoForma),
    /// `0x3f/0x19`: 0 FPV, 1 kit de programação, 2 só chassi, 3 óculos.
    ModoChassi(u8),
    /// `0x3f/0x20`, em rpm com sinal de andar para a frente, na ordem frente-direita, frente-esquerda,
    /// trás-esquerda, trás-direita. A DJI inverte a roda 2 e a 3 antes de enviar (`rm_ctrl.py:394`).
    Rodas([i16; 4]),
    /// `0x3f/0x22`: m/s e m/s, e o ângulo em graus entre chassi e torre.
    Seguir { vx: f32, vy: f32, angulo: f32 },
    /// `0x3f/0x28`: 0 desligado, 1 ligado, 2 ligado com eixos.
    Sobrepor(u8),
    /// `0x04/0x4c`: 0 livre, 1 FPV, 2 seguindo o chassi.
    ModoTorre(u8),
    /// `0x3f/0xb0`, em décimos de grau, com o sinal da DJI: yaw à direita, pitch para cima.
    TorreAngulos {
        yaw_ddeg: i16,
        pitch_ddeg: i16,
        /// Bit 0 yaw, bit 2 pitch (`rm_define.gimbal_axis_*_maskbit`).
        eixos: u8,
        /// 4 é yaw no chassi e pitch NED, o padrão de `angle_ctrl` (`rm_ctrl.py:1662`).
        coordenada: u8,
        velocidade: u16,
    },
}

/// Bits de eixo de `0x3f/0xb0`.
pub const EIXO_YAW: u8 = 0x01;
pub const EIXO_PITCH: u8 = 0x04;
/// `task_ctrl` da DJI: início, com progresso a 10 Hz (`TASK_FREQ_10Hz << 2`).
const TAREFA_INICIO_10HZ: u8 = 0x08;

impl Comando {
    pub fn id(&self) -> (u8, u8) {
        match self {
            Self::Permissao(_) => id::PERMISSAO,
            Self::ModoChassi(_) => id::MODO_CHASSI,
            Self::Rodas(_) => id::RODAS,
            Self::Seguir { .. } => id::SEGUIR,
            Self::Sobrepor(_) => id::SOBREPOR,
            Self::ModoTorre(_) => id::MODO_TORRE,
            Self::TorreAngulos { .. } => id::TORRE_ANGULOS,
        }
    }

    pub fn quadro(&self, nos: &Nos, seq: u16) -> Quadro {
        let (origem, destino, dados): (u8, u8, Vec<u8>) = match *self {
            Self::Permissao(PermissaoForma::Dji) => (
                nos.remetente_alavanca,
                nos.chassi,
                ALAVANCA_NEUTRA_DJI.to_vec(),
            ),
            Self::Permissao(PermissaoForma::Proroklab) => (
                nos.remetente_alavanca,
                byte_do_no(300),
                ALAVANCA_NEUTRA_PROROKLAB.to_vec(),
            ),
            Self::ModoChassi(m) => (nos.remetente, nos.chassi, vec![m]),
            Self::Rodas([fr, fl, bl, br]) => {
                let mut d = Vec::with_capacity(8);
                for v in [fr, fl.wrapping_neg(), bl.wrapping_neg(), br] {
                    d.extend_from_slice(&v.to_le_bytes());
                }
                (nos.remetente, nos.chassi, d)
            }
            Self::Seguir { vx, vy, angulo } => {
                let mut d = Vec::with_capacity(12);
                for v in [vx, vy, angulo] {
                    d.extend_from_slice(&v.to_le_bytes());
                }
                (nos.remetente, nos.chassi, d)
            }
            Self::Sobrepor(m) => (nos.remetente, nos.chassi, vec![m]),
            Self::ModoTorre(m) => (nos.remetente, nos.torre, vec![m, 0]),
            Self::TorreAngulos {
                yaw_ddeg,
                pitch_ddeg,
                eixos,
                coordenada,
                velocidade,
            } => {
                let mut d = vec![0, TAREFA_INICIO_10HZ, (coordenada << 3) | (eixos & 0x07)];
                for v in [yaw_ddeg, 0, pitch_ddeg] {
                    d.extend_from_slice(&v.to_le_bytes());
                }
                for v in [0, velocidade, 0, velocidade] {
                    d.extend_from_slice(&u16::to_le_bytes(v));
                }
                (nos.remetente, nos.torre, d)
            }
        };
        let (cmd_set, cmd_id) = self.id();
        Quadro {
            origem,
            destino,
            seq,
            resposta: false,
            pede: PedeAck::Nada,
            cmd_set,
            cmd_id,
            dados,
        }
    }
}

/// Uma sequência por comando, como o robô e a DJI fazem.
#[derive(Debug, Default)]
pub struct SeqPorComando(HashMap<(u8, u8), u16>);

impl SeqPorComando {
    pub fn proxima(&mut self, cmd: (u8, u8)) -> u16 {
        let s = self.0.entry(cmd).or_insert(0);
        let atual = *s;
        *s = s.wrapping_add(1);
        atual
    }
}
