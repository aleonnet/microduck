//! O agente do S1: cumpre no corpo o pedido que a estação Hailo validou.
//!
//! - [`tabela`]: as habilidades, lidas de um arquivo: para cada ferramenta, o que pedir ao `rms1d`, o limite, e quando
//!   termina.
//! - [`agente`]: o núcleo, sem rede: entram o pedido validado, o estado do corpo e as detecções; saem o que pedir ao
//!   `rms1d` e como cada habilidade terminou.
//! - [`ligacoes`]: os dois sockets locais, o do `rms1-mediad` (de onde vêm o pedido e as detecções) e o do `rms1d` (que é
//!   quem move).
//!
//! O desenho, com o que cada habilidade faz: `docs/s1/design/rms1-agente.md`.

pub mod agente;
pub mod ligacoes;
pub mod tabela;
