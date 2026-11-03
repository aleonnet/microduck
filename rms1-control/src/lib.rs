//! O controle do S1, sem soquete e sem tokio.
//!
//! Invariantes, do desenho do pato (`docs/design/robotd-design.md`, 1.5), e quem os garante:
//!
//! 1. **Nada acima de [`seguranca::Seguranca`] escreve no corpo.** Ela é dona do único [`corpo::Corpo`], e o
//!    compilador garante.
//! 2. **Ligar o processo não move o robô.** Desarmada, ela só envia o que a telemetria pede, e nada disso move.
//! 3. **O laço nunca espera cliente.** Intenções são leituras atômicas; o estado só é montado se pedido.
//! 4. **Saúde é publicada, nunca perguntada.** Um laço travado se denuncia pela idade do último ciclo.

pub mod cinematica;
pub mod corpo;
pub mod gravacao;
pub mod intencoes;
pub mod laco;
pub mod saude;
pub mod seguranca;
pub mod telemetria;

pub use corpo::{Corpo, CorpoFalso, CorpoRede, CorpoSerial};
pub use intencoes::{Intencoes, Modo};
pub use laco::{Estado, Laco};
pub use saude::Saude;
pub use seguranca::Seguranca;
pub use telemetria::{Amostra, Enlace};
