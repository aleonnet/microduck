//! O serviço de mídia do S1: câmera, microfone e alto-falante, com um dono só.
//!
//! No pato a câmera é do `mediad` e o microfone e o som são do `robotd`. No S1 os três ficam aqui, num processo fora do
//! `rms1d`: a saída de som e a captura do microfone são de um cliente só, a trava contra a própria voz precisa saber, no
//! mesmo lugar, o que toca e o que chega, e uma falha de mídia não pode derrubar quem move o robô.
//!
//! Nada aqui depende do `mediad` do pato, que só existe com GStreamer. O que é igual ao pato é a interface: `media.frame`
//! com o cabeçalho do contrato, e o encontro de quadros, refeito em [`quadros`].

pub mod camera;
pub mod quadros;
pub mod servidor;
