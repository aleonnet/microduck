//! O que o corpo diz ao serviço de mídia: a torre e a pose, lidas do `rms1d`.
//!
//! Para situar no chão o que a câmera vê, o serviço precisa saber para onde a lente olhava quando o quadro foi tirado. Quem
//! sabe é o `rms1d`, que publica a torre e a pose em `s1.state`. Este cliente só lê: liga ao socket de `[midia] robo`, assina
//! `s1.subscribe`, e leva cada estado a quem o chamou como uma amostra, com a hora em que ela chegou. Não diz nome nem pede
//! canal: para o `rms1d` é a fonte sem nome, que não manda nada.
//!
//! A conexão que cai religa sozinha, com espera crescente. Enquanto não há amostra, quem as usa diz que não tem a pose.

use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rms1_mundo::Amostra;
use serde_json::Value;

/// Quantas amostras por segundo o cliente pede: o ritmo em que o corpo do S1 manda a telemetria (um pacote a cada 20 ms:
/// `rms1d-params`, `Telemetria`), e o da história do cérebro do pato (`autod/src/obstacle.rs`, um segundo a 50 Hz).
pub const AMOSTRAS_HZ: u32 = 50;
const ESPERA_MIN: Duration = Duration::from_millis(100);
const ESPERA_MAX: Duration = Duration::from_secs(2);
/// De quanto em quanto a leitura confere se é hora de parar.
const CONFERIR: Duration = Duration::from_millis(200);
/// A maior linha que o cliente lê: um `s1.state` tem menos de 2 kB. A que passa disto não é do `rms1d`, e a ligação recomeça.
const LINHA_MAXIMA: usize = 64 * 1024;

/// O relógio das amostras e das capturas: segundos contados de quando o serviço subiu, que nunca voltam.
#[derive(Debug, Clone, Copy)]
pub struct Relogio(Instant);

impl Default for Relogio {
    fn default() -> Self {
        Self(Instant::now())
    }
}

impl Relogio {
    pub fn agora(&self) -> f64 {
        self.de(Instant::now())
    }

    /// Um instante neste relógio. O de antes de o relógio começar vale zero.
    pub fn de(&self, instante: Instant) -> f64 {
        instante.saturating_duration_since(self.0).as_secs_f64()
    }
}

/// A amostra de um `s1.state`, chegado na hora `t`. O `rms1d` publica graus, com o giro e o rumo positivos à esquerda e a
/// inclinação positiva para cima (`rms1-control/src/telemetria.rs`); a amostra leva radianos. Da torre vão o giro em relação
/// ao chassi e a inclinação em relação ao chão. Sem pacote recente do corpo, ou sem a torre ou a pose, não há amostra: um
/// número velho situaria o quadro no lugar errado.
pub fn amostra_de(params: &Value, t: f64) -> Option<Amostra> {
    if params["link"]["fresh"].as_bool() != Some(true) {
        return None;
    }
    let numero = |v: &Value| v.as_f64().filter(|n| n.is_finite());
    let corpo = &params["body"];
    let torre = &params["gimbal"];
    Some(Amostra {
        t,
        torre: [
            numero(&torre["yaw_deg"])?.to_radians(),
            numero(&torre["pitch_ground_deg"])?.to_radians(),
        ],
        pose: [
            numero(&corpo["position_m"][0])?,
            numero(&corpo["position_m"][1])?,
            numero(&corpo["attitude_deg"]["yaw"])?.to_radians(),
        ],
    })
}

/// Dorme `por`, em pedaços, e diz se é hora de parar.
fn esperar(por: Duration, parar: &AtomicBool) -> bool {
    let fim = Instant::now() + por;
    while let Some(resta) = fim.checked_duration_since(Instant::now()) {
        if parar.load(Ordering::Relaxed) {
            return true;
        }
        std::thread::sleep(resta.min(Duration::from_millis(20)));
    }
    parar.load(Ordering::Relaxed)
}

/// Roda até `parar`: mantém a ligação com o `rms1d` em `socket` e leva cada amostra a `receber`.
pub fn rodar(
    socket: &Path,
    relogio: Relogio,
    mut receber: impl FnMut(Amostra),
    parar: Arc<AtomicBool>,
) {
    let assinar = serde_json::json!({"jsonrpc": "2.0", "id": 0, "method": "s1.subscribe", "params": {"hz": AMOSTRAS_HZ}});
    let assinar = format!("{assinar}\n");
    let mut espera = ESPERA_MIN;
    while !parar.load(Ordering::Relaxed) {
        let ligada = UnixStream::connect(socket).and_then(|mut c| {
            c.set_read_timeout(Some(CONFERIR))?;
            c.write_all(assinar.as_bytes())?;
            Ok(c)
        });
        let Ok(conexao) = ligada else {
            if esperar(espera, &parar) {
                return;
            }
            espera = (espera * 2).min(ESPERA_MAX);
            continue;
        };
        tracing::info!(socket = %socket.display(), "ligado ao rms1d, para a torre e a pose");
        let mut leitor = BufReader::new(conexao);
        // A linha fica fora do laço: o prazo da leitura pode vencer no meio dela, e o que já chegou não se perde.
        let mut linha = Vec::new();
        loop {
            if parar.load(Ordering::Relaxed) {
                return;
            }
            // O limite vale antes de a linha ser guardada: este processo não segura uma linha sem fim.
            let resto = (LINHA_MAXIMA + 1).saturating_sub(linha.len()) as u64;
            match (&mut leitor).take(resto).read_until(b'\n', &mut linha) {
                // O `rms1d` fechou.
                Ok(0) => break,
                Ok(_) if linha.ends_with(b"\n") => {
                    let t = relogio.agora();
                    let lida = serde_json::from_slice::<Value>(&linha).ok();
                    linha.clear();
                    let Some(lida) = lida else { continue };
                    if let Some(recusa) = lida["error"]["message"].as_str() {
                        tracing::warn!(recusa, "o rms1d recusou a assinatura da torre e da pose");
                    }
                    if lida["method"] == "s1.state"
                        && let Some(a) = amostra_de(&lida["params"], t)
                    {
                        // A conexão que dá amostra provou que serve: a espera de religar volta ao mínimo.
                        espera = ESPERA_MIN;
                        receber(a);
                    }
                }
                // O fim da conexão no meio de uma linha, ou a linha grande demais.
                Ok(_) => break,
                // O prazo da leitura venceu: é só a hora de conferir se é para parar.
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(_) => break,
            }
        }
        tracing::warn!(socket = %socket.display(), "a ligação com o rms1d caiu");
        if esperar(espera, &parar) {
            return;
        }
        espera = (espera * 2).min(ESPERA_MAX);
    }
}
