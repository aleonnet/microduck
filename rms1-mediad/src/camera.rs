//! As fontes da câmera, atrás da mesma interface: a de teste, a do simulador e a do robô.
//!
//! Uma fonte roda numa thread e só copia um quadro quando [`Quadros::levar_pedido`] diz que alguém pediu.

use crate::quadros::{Quadro, Quadros};

/// O tamanho da fonte de teste: o do `Source::Test` do pato (`mediad/src/pipeline.rs:456-462`), pequeno para não gastar CPU.
pub const TESTE_LARGURA: u32 = 256;
pub const TESTE_ALTURA: u32 = 144;
pub const NV12: &str = "NV12";

/// Quantos bytes tem um quadro NV12: a luma inteira e a croma pela metade nas duas direções.
pub const fn bytes_nv12(largura: u32, altura: u32) -> usize {
    (largura as usize) * (altura as usize) * 3 / 2
}

/// O quadro de teste de número `n`: uma rampa de luma que anda uma coluna por quadro, com a croma neutra. Determinístico:
/// o teste confere os bytes exatos.
pub fn quadro_de_teste(n: u64) -> Quadro {
    let (l, a) = (TESTE_LARGURA as usize, TESTE_ALTURA as usize);
    let mut dados = vec![128u8; bytes_nv12(TESTE_LARGURA, TESTE_ALTURA)];
    let fase = (n % TESTE_LARGURA as u64) as usize;
    for y in 0..a {
        for x in 0..l {
            // De 16 a 235, a faixa limitada do BT.601, que é a do quadro do robô.
            dados[y * l + x] = 16 + (((x + fase) % l) * 219 / (l - 1)) as u8;
        }
    }
    Quadro {
        largura: TESTE_LARGURA,
        altura: TESTE_ALTURA,
        formato: NV12,
        capturado_em: std::time::SystemTime::now(),
        dados,
    }
}

/// A fonte de teste, até `parar` ficar verdadeiro: um quadro a cada `periodo`, copiado só sob pedido.
pub fn rodar_teste(
    quadros: Quadros,
    periodo: std::time::Duration,
    parar: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let mut n = 0u64;
    while !parar.load(std::sync::atomic::Ordering::Relaxed) {
        if quadros.levar_pedido() {
            quadros.entregar(quadro_de_teste(n));
        }
        n = n.wrapping_add(1);
        std::thread::sleep(periodo);
    }
}
