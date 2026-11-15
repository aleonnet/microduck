//! As fontes da câmera, atrás da mesma interface: a de teste, a do simulador e a do robô.
//!
//! Uma fonte roda numa thread e só copia um quadro quando [`Quadros::levar_pedido`] diz que alguém pediu.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::quadros::{Quadro, Quadros};

/// O tamanho da fonte de teste: o do `Source::Test` do pato (`mediad/src/pipeline.rs:456-462`), pequeno para não gastar CPU.
pub const TESTE_LARGURA: u32 = 256;
pub const TESTE_ALTURA: u32 = 144;
pub const NV12: &str = "NV12";
/// O quadro da câmera do robô, que a do simulador também entrega: NV12, 1280 por 720, 1.382.400 bytes.
pub const S1_LARGURA: u32 = 1280;
pub const S1_ALTURA: u32 = 720;
/// O pedido de um quadro ao simulador: 4 bytes de zero. A resposta é o formato da câmera simulada do pato
/// (`mediad/src/pipeline.rs`, `read_frames`): 4 bytes de tamanho e o quadro.
const PEDIDO_AO_SIMULADOR: [u8; 4] = [0; 4];
/// Quanto a câmera espera a resposta do simulador: menos do que quem pediu o quadro espera por ele.
const PRAZO_DO_SIMULADOR: Duration = Duration::from_millis(450);
/// De quanto em quanto a câmera olha se alguém pediu um quadro.
const OLHAR_O_PEDIDO: Duration = Duration::from_millis(2);

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

/// Pede um quadro ao simulador e o lê. Quadro de outro tamanho é erro, com os dois tamanhos: os dois lados discordam da
/// geometria, e um quadro assim ninguém lê.
fn pedir_ao_simulador(conexao: &mut TcpStream, esperado: usize) -> std::io::Result<Vec<u8>> {
    conexao.write_all(&PEDIDO_AO_SIMULADOR)?;
    let mut tamanho = [0u8; 4];
    conexao.read_exact(&mut tamanho)?;
    let tamanho = u32::from_le_bytes(tamanho) as usize;
    if tamanho != esperado {
        return Err(std::io::Error::other(format!(
            "veio um quadro de {tamanho} bytes, e o da câmera do S1 tem {esperado}"
        )));
    }
    let mut dados = vec![0u8; esperado];
    conexao.read_exact(&mut dados)?;
    Ok(dados)
}

/// A câmera do simulador, até `parar` ficar verdadeiro. O quadro é pedido ao serviço local do simulador, em `endereco`, só
/// quando alguém o quer: sem pedido, a página não desenha nada, como a câmera do robô não copia.
///
/// `dizer` recebe o motivo de a câmera não entregar quadro, e `None` quando ela volta a entregar; é o que
/// `s1.media.status` mostra. Conexão recusada ou caída é refeita depois de `espera`, e a câmera segue tentando: o simulador
/// reinicia, e isso não pode derrubar o serviço.
pub fn rodar_simulador(
    quadros: Quadros,
    endereco: &str,
    espera: Duration,
    dizer: impl Fn(Option<String>),
    parar: Arc<AtomicBool>,
) {
    let esperado = bytes_nv12(S1_LARGURA, S1_ALTURA);
    // O que foi dito por último: o mesmo motivo não é repetido a cada tentativa.
    let mut dito: Option<Option<String>> = None;
    let mut avisar = |novo: Option<String>| {
        if dito.as_ref() != Some(&novo) {
            match &novo {
                Some(motivo) => tracing::warn!(motivo, "a câmera do simulador não entrega quadro"),
                None => tracing::info!(endereco, "a câmera do simulador está entregando"),
            }
            dizer(novo.clone());
            dito = Some(novo);
        }
    };
    while !parar.load(Ordering::Relaxed) {
        let mut conexao = match TcpStream::connect(endereco) {
            Ok(c) => c,
            Err(e) => {
                avisar(Some(format!(
                    "sem a câmera do simulador em {endereco}: {e}"
                )));
                // Quem pediu já desistiu no prazo dele: o pedido não fica guardado para um quadro que ninguém espera.
                quadros.levar_pedido();
                std::thread::sleep(espera);
                continue;
            }
        };
        let _ = conexao.set_nodelay(true);
        let _ = conexao.set_read_timeout(Some(PRAZO_DO_SIMULADOR));
        while !parar.load(Ordering::Relaxed) {
            if !quadros.levar_pedido() {
                std::thread::sleep(OLHAR_O_PEDIDO);
                continue;
            }
            match pedir_ao_simulador(&mut conexao, esperado) {
                Ok(dados) => {
                    avisar(None);
                    quadros.entregar(Quadro {
                        largura: S1_LARGURA,
                        altura: S1_ALTURA,
                        formato: NV12,
                        capturado_em: std::time::SystemTime::now(),
                        dados,
                    });
                }
                Err(e) => {
                    // Depois de um erro não se sabe em que ponto da conversa a conexão ficou: ela é refeita.
                    avisar(Some(format!("a câmera do simulador em {endereco}: {e}")));
                    break;
                }
            }
        }
        std::thread::sleep(espera);
    }
}
