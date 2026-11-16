//! O cliente do `rms1-venc`, o programa do lado Android do robô que comanda o codificador de hardware do S1 (Hantro H1).
//!
//! O `rms1-mediad` roda no chroot e não carrega as bibliotecas do codificador, que são do Android: o `rms1-venc`
//! (`rms1-mediad/venc/rms1-venc.c`) as chama, e os dois conversam por um socket de domínio Unix que os dois lados enxergam
//! (`[midia] venc`, no robô `/run/rms1-venc/venc.sock`). Por ele vai o quadro NV12 de 640 por 360 e volta o quadro em
//! H.264 ou em JPEG, feito pelo chip em 2 a 3 ms; por software, no mesmo processador, o JPEG leva de 83 a 89 ms (medido no
//! robô em 2026-11-15).
//!
//! **Uma conexão só.** O `rms1-venc` atende um cliente por vez, e o chip é um só: a percepção e a imagem da tela usam o mesmo
//! [`Venc`], atrás de um `Mutex`. **Um pedido que não volta no prazo derruba a conexão,** e o pedido seguinte a refaz.
//!
//! O protocolo (o cabeçalho do `rms1-venc.c` o descreve): o pedido leva 1 byte de comando, 4 de parâmetro e 4 de tamanho,
//! little-endian, e a carga; a resposta, 1 byte de estado (0 = certo), 4 de tamanho, e a carga: o quadro codificado, ou o
//! motivo em texto.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

/// O quadro que o chip recebe: NV12 deste tamanho.
pub const LARGURA: usize = 640;
pub const ALTURA: usize = 360;
/// Os bytes de um quadro NV12 desse tamanho.
pub const QUADRO: usize = LARGURA * ALTURA * 3 / 2;
/// Quanto se espera por uma resposta. O chip leva de 2 a 3 ms por quadro; um fluxo novo, que refaz o codificador, mais.
const PRAZO: Duration = Duration::from_millis(500);
/// A maior resposta aceita: a memória de saída do `rms1-venc` tem 512 KiB, e os cabeçalhos do fluxo vão na frente.
const RESPOSTA_MAXIMA: usize = 1024 * 1024;

const QUADRO_H264: u8 = 1;
const QUADRO_JPEG: u8 = 2;
const FLUXO_NOVO: u8 = 3;

pub struct Venc {
    caminho: PathBuf,
    conexao: Option<UnixStream>,
}

impl Venc {
    pub fn novo(caminho: impl Into<PathBuf>) -> Self {
        Self {
            caminho: caminho.into(),
            conexao: None,
        }
    }

    /// Começa um fluxo de H.264: `mbps` é a taxa (0 = o máximo do chip), `fps` os quadros por segundo. O quadro seguinte
    /// sai inteiro, com os cabeçalhos na frente.
    pub fn fluxo(&mut self, mbps: u32, fps: u32) -> Result<(), String> {
        self.pedir(FLUXO_NOVO, (mbps & 0xffff) | (fps.min(0xffff) << 16), &[])
            .map(|_| ())
    }

    /// Um quadro NV12 de 640 por 360 em H.264, no fluxo aberto por [`Venc::fluxo`].
    pub fn h264(&mut self, nv12: &[u8]) -> Result<Vec<u8>, String> {
        self.pedir(QUADRO_H264, 0, nv12)
    }

    /// Um quadro NV12 de 640 por 360 em JPEG, no nível dado (0 a 10).
    pub fn jpeg(&mut self, nv12: &[u8], nivel: u32) -> Result<Vec<u8>, String> {
        self.pedir(QUADRO_JPEG, nivel, nv12)
    }

    fn pedir(&mut self, comando: u8, param: u32, carga: &[u8]) -> Result<Vec<u8>, String> {
        if !carga.is_empty() && carga.len() != QUADRO {
            return Err(format!(
                "o quadro para o codificador tem {} bytes, e ele recebe NV12 de {LARGURA} por {ALTURA} ({QUADRO})",
                carga.len()
            ));
        }
        if self.conexao.is_none() {
            let c = UnixStream::connect(&self.caminho).map_err(|e| {
                format!("o rms1-venc não atende em {}: {e}", self.caminho.display())
            })?;
            let _ = c.set_read_timeout(Some(PRAZO));
            let _ = c.set_write_timeout(Some(PRAZO));
            self.conexao = Some(c);
        }
        match self.trocar(comando, param, carga) {
            Ok(resposta) => resposta,
            Err(e) => {
                // A conexão não se sabe mais em que ponto está: o pedido seguinte a refaz.
                self.conexao = None;
                Err(format!("o rms1-venc não respondeu: {e}"))
            }
        }
    }

    /// Manda o pedido e lê a resposta. O erro de fora é da conexão; o de dentro, do codificador, com o motivo dele.
    fn trocar(
        &mut self,
        comando: u8,
        param: u32,
        carga: &[u8],
    ) -> std::io::Result<Result<Vec<u8>, String>> {
        let c = self.conexao.as_mut().expect("conexão aberta acima");
        let mut cab = [0u8; 9];
        cab[0] = comando;
        cab[1..5].copy_from_slice(&param.to_le_bytes());
        cab[5..9].copy_from_slice(&(carga.len() as u32).to_le_bytes());
        c.write_all(&cab)?;
        c.write_all(carga)?;
        let mut r = [0u8; 5];
        c.read_exact(&mut r)?;
        let tamanho = u32::from_le_bytes([r[1], r[2], r[3], r[4]]) as usize;
        if tamanho > RESPOSTA_MAXIMA {
            return Err(std::io::Error::other(format!(
                "resposta de {tamanho} bytes, acima do máximo"
            )));
        }
        let mut dados = vec![0u8; tamanho];
        c.read_exact(&mut dados)?;
        Ok(if r[0] == 0 {
            Ok(dados)
        } else {
            Err(format!(
                "o codificador recusou: {}",
                String::from_utf8_lossy(&dados)
            ))
        })
    }
}
