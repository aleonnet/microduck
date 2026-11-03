//! O protocolo do corpo do S1: quadro, verificadores, comandos e telemetria.
//!
//! A origem de cada byte é a biblioteca do laboratório e o firmware copiado do robô. Cada comando
//! cita a mensagem da biblioteca. Os testes em `tests/` conferem contra quadros lidos da UART do robô.

pub mod barramento;
pub mod comando;
pub mod crc;
pub mod quadro;

pub use comando::{Comando, Nos, PermissaoForma, SeqPorComando, id};
pub use quadro::{Leitor, PedeAck, Quadro, QuadroErro};
