//! O mixer do chip de áudio do S1: o serviço grava o estado que a partida da DJI deixa, e confere lendo de volta.
//!
//! Com a DJI parada ninguém mais configura o chip, e o microfone e o alto-falante dependem dele. O estado é o que foi
//! lido no firmware em 2026-11-13: as 18 linhas de `tinymix` do `start_dji_system.sh` ([`PARTIDA`]) e, por cima, os seis
//! controles que o `dji_camera` grava em `init_audio_info` ([`DO_DJI_CAMERA`]). Os valores finais batem com o mixer medido
//! com a DJI viva (`out/2026-10-31-poc2/mic-lab/tinymix_stock_alive_app_closed.txt`, no laboratório).
//!
//! **Um valor que não pega não passa calado:** [`configurar`] para no primeiro controle cuja leitura difere do gravado, e
//! diz qual. O serviço não sobe com o chip num estado que ele não conhece.
//!
//! O volume é o da DJI ([`volume`]): o `Stereo DAC Gain`, na conta `v × 127 / 100`, com o mudo no `Stereo DAC Mute`
//! (`libduml_hal.so`, `p1_speaker.c`, função `0x1aea0`). O `Speaker Volume` fica nos 20 da partida.
//!
//! Os números são os do `tinymix`, que conta de 0. No robô quem grava é o `amixer` do chroot, que conta de 1 ([`Amixer`]):
//! cada leitura confere também o nome do controle, e um nome diferente do esperado é recusado.

use std::io;

/// Um controle e o valor de cada canal dele. Opção de lista vai pelo índice; chave vai como 0 ou 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Linha {
    pub id: u32,
    pub nome: &'static str,
    pub valores: &'static [u32],
}

const fn linha(id: u32, nome: &'static str, valores: &'static [u32]) -> Linha {
    Linha { id, nome, valores }
}

/// As 18 linhas de `tinymix` do `start_dji_system.sh`, na ordem do script: o alto-falante e depois o microfone.
pub const PARTIDA: [Linha; 18] = [
    linha(0, "DACR Input", &[1]), // I2SR
    linha(1, "DACL Input", &[0]), // I2SL
    linha(2, "Stereo DAC Mute", &[1, 0]),
    linha(5, "Speaker Volume", &[20]),
    linha(6, "Headphone Volume", &[25, 25]),
    linha(11, "Stereo DAC Gain", &[103, 103]),
    linha(40, "Aux Mixer Right Playback Switch", &[1]),
    linha(45, "Spk Mixer Right Playback Switch", &[0]),
    linha(14, "MIC1 Volume", &[35]),
    linha(15, "MIC2 Volume", &[35]),
    linha(21, "Pcm To Codec", &[1]),
    linha(25, "Adc2 Mixer Mic2 Input Switch", &[1]),
    linha(26, "Adc1 Mixer Mic1 Input Switch", &[1]),
    linha(50, "I2sR Out", &[0]),    // ADC1
    linha(51, "I2sL Out", &[1]),    // ADC2
    linha(54, "mic1Pga mux", &[0]), // MIC1
    linha(55, "sideTn mux", &[0]),  // MIC1PGA
    linha(56, "mic2Pga mux", &[0]), // MIC2
];

/// Os seis controles do microfone que o `dji_camera` grava na partida, com o padrão do binário (`.data`, `0x111550`),
/// que é igual ao `audio.ini` do robô nesses bytes.
pub const DO_DJI_CAMERA: [Linha; 6] = [
    linha(12, "ADC1 Gain", &[103]),
    linha(13, "ADC2 Gain", &[103]),
    linha(19, "MIC1 Boost", &[0]),
    linha(20, "MIC2 Boost", &[1]),
    linha(14, "MIC1 Volume", &[14]),
    linha(15, "MIC2 Volume", &[29]),
];

/// O mudo e o ganho do alto-falante: os dois controles em que o volume mexe.
pub const MUDO: Linha = linha(2, "Stereo DAC Mute", &[0, 0]);
pub const GANHO: Linha = linha(11, "Stereo DAC Gain", &[0, 0]);
/// O volume com que o serviço sobe: 57, que dá os 72 medidos no `Stereo DAC Gain` com a DJI viva.
pub const VOLUME_DE_PARTIDA: u8 = 57;

/// O que um controle respondeu à leitura: o nome e o valor de cada canal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lido {
    pub nome: String,
    pub valores: Vec<u32>,
}

/// Os controles do chip: o `amixer` no robô, e um de mentira no teste.
pub trait Controles: Send {
    fn gravar(&mut self, id: u32, valores: &[u32]) -> io::Result<()>;
    fn ler(&mut self, id: u32) -> io::Result<Lido>;
}

/// Grava um controle e confere lendo de volta: o nome e cada canal.
fn gravar_e_conferir(
    c: &mut dyn Controles,
    id: u32,
    nome: &str,
    valores: &[u32],
) -> Result<(), String> {
    c.gravar(id, valores)
        .map_err(|e| format!("o mixer recusou gravar `{nome}` (controle {id}): {e}"))?;
    let lido = c
        .ler(id)
        .map_err(|e| format!("o mixer não deixou ler `{nome}` (controle {id}): {e}"))?;
    if lido.nome != nome {
        return Err(format!(
            "o controle {id} do mixer se chama `{}`, e o esperado é `{nome}`: nada mais foi gravado",
            lido.nome
        ));
    }
    if lido.valores != valores {
        return Err(format!(
            "o valor não pegou em `{nome}` (controle {id}): gravei {valores:?} e li {:?}",
            lido.valores
        ));
    }
    Ok(())
}

/// Deixa o chip como a partida da DJI deixa: as 18 linhas e, por cima, os seis controles. Para no primeiro que falha.
pub fn configurar(c: &mut dyn Controles) -> Result<(), String> {
    for l in PARTIDA.iter().chain(DO_DJI_CAMERA.iter()) {
        gravar_e_conferir(c, l.id, l.nome, l.valores)?;
    }
    Ok(())
}

/// O ganho que um volume de 0 a 100 dá: a conta da DJI, `v × 127 / 100`.
pub fn ganho_do_volume(v: u8) -> u32 {
    u32::from(v.min(100)) * 127 / 100
}

/// O volume do alto-falante, de 0 a 100. Zero só liga o mudo; outro valor tira o mudo e grava o ganho nos dois canais.
pub fn volume(c: &mut dyn Controles, v: u8) -> Result<(), String> {
    if v == 0 {
        return gravar_e_conferir(c, MUDO.id, MUDO.nome, &[1, 1]);
    }
    gravar_e_conferir(c, MUDO.id, MUDO.nome, &[0, 0])?;
    let g = ganho_do_volume(v);
    gravar_e_conferir(c, GANHO.id, GANHO.nome, &[g, g])
}

/// Lê a resposta de `amixer cget`: o nome na primeira linha e os valores na linha `: values=`. Chave vem como `on` e `off`.
pub fn ler_cget(texto: &str) -> Option<Lido> {
    let nome = texto
        .lines()
        .find_map(|l| l.split_once("name='"))
        .and_then(|(_, r)| r.rsplit_once('\''))
        .map(|(n, _)| n.to_owned())?;
    let valores = texto
        .lines()
        .find_map(|l| l.trim_start().strip_prefix(": values="))?
        .trim()
        .split(',')
        .map(|v| match v {
            "on" => Some(1),
            "off" => Some(0),
            n => n.parse().ok(),
        })
        .collect::<Option<Vec<u32>>>()?;
    Some(Lido { nome, valores })
}

/// O `amixer` do chroot do robô, na placa 0. Ele conta os controles de 1: o do `tinymix` mais um.
pub struct Amixer;

impl Amixer {
    fn rodar(args: &[String]) -> io::Result<String> {
        let saida = std::process::Command::new("amixer")
            .args(["-c", "0"])
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()?;
        if !saida.status.success() {
            return Err(io::Error::other(format!(
                "amixer {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&saida.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&saida.stdout).into_owned())
    }
}

impl Controles for Amixer {
    fn gravar(&mut self, id: u32, valores: &[u32]) -> io::Result<()> {
        let v: Vec<String> = valores.iter().map(u32::to_string).collect();
        Self::rodar(&["cset".into(), format!("numid={}", id + 1), v.join(",")]).map(|_| ())
    }

    fn ler(&mut self, id: u32) -> io::Result<Lido> {
        let texto = Self::rodar(&["cget".into(), format!("numid={}", id + 1)])?;
        ler_cget(&texto)
            .ok_or_else(|| io::Error::other(format!("não entendi a resposta do amixer: {texto}")))
    }
}
