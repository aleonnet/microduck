//! `rms1d.toml` e a tabela física do S1.
//!
//! O S1 é um só: todo número físico mora em `robo_s1.json`, com origem e prova, e é o mesmo para o `rms1d`, o simulador
//! e o treino. `rms1d.toml` fica com o que não é físico, e número físico nele é recusado pelo nome. Arquivo ausente usa
//! os padrões; chave desconhecida é recusada pelo nome, porque um `prazo_ms` escrito errado deixaria o robô com o prazo
//! padrão sem ninguém saber.

use std::path::{Path, PathBuf};

use serde::Deserialize;

pub const CAMINHO_PADRAO: &str = "/etc/robot/rms1d.toml";

/// A tabela física do S1, a origem única.
pub const TABELA: &str = include_str!("../robo_s1.json");

/// O único que pode ser parado no robô: os dois que seguram a serial, e o `dji_camera`, que segura a câmera e o microfone
/// e é parado para o `rms1-mediad` ficar com eles (a bancada o para por `stop` e `setprop ctl.stop`, sem `kill`: morto no
/// meio do fluxo, ele deixou a câmera sem responder até reiniciar). Lista de permitidos, e não de proibidos: parar
/// `dji_scratch` derrubou `dji_sys` (`docs/s1/robot/rms1d-pontos-de-controle.md`), e o próximo nome ninguém conhece.
const PODE_PARAR: [&str; 3] = ["dji_monitor", "dji_hdvt_uav", "dji_camera"];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Params {
    pub robo: Robo,
    pub corpo: Corpo,
    pub nos: Nos,
    pub laco: Laco,
    pub seguranca: Seguranca,
    /// Da tabela física: não vem de `rms1d.toml`.
    #[serde(skip)]
    pub geometria: Geometria,
    pub torre: Torre,
    pub telemetria: Telemetria,
    pub estimativa: Estimativa,
    pub armadura: Armadura,
    pub mira: Mira,
    pub dji: Dji,
    /// O serviço de mídia, `rms1-mediad`. Sem a seção, o `rms1d` não repassa som e recusa dizendo isso.
    pub midia: Option<Midia>,
    /// Quem manda no corpo: uma linha por fonte. Escrita no arquivo, a tabela substitui a padrão inteira.
    pub fontes: Vec<Fonte>,
    pub autonomia: Autonomia,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            robo: Robo::default(),
            corpo: Corpo::default(),
            nos: Nos::default(),
            laco: Laco::default(),
            seguranca: Seguranca::default(),
            geometria: Geometria::default(),
            torre: Torre::default(),
            telemetria: Telemetria::default(),
            estimativa: Estimativa::default(),
            armadura: Armadura::default(),
            mira: Mira::default(),
            dji: Dji::default(),
            midia: None,
            fontes: Fonte::padrao(),
            autonomia: Autonomia::default(),
        }
    }
}

/// O nome da fonte de quem não diz o nome no `hello`, ou diz um que a tabela não tem. Toda tabela tem de trazê-la.
pub const FONTE_SEM_NOME: &str = "sem_nome";
/// O teto de linhas da tabela: a arbitragem percorre todas a cada ciclo do laço.
pub const FONTES_MAX: usize = 16;

/// O que uma fonte pode pedir ao corpo. Cada pedido do contrato cai em um.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Canal {
    /// `robot.move`.
    Rodas,
    /// `robot.head`, `robot.look` e `s1.gimbal`.
    Torre,
    /// `robot.sound`.
    Som,
    /// Armar e desarmar: `robot.enable`, `robot.init` e `robot.relax`.
    Energia,
    /// Quem segue quem: `robot.do`.
    Modo,
    /// O que fica gravado no robô: `s1.armor` e `s1.sight` com valores.
    Ajustes,
    /// Ligar e desligar o robô por conta própria: `robot.autonomous`.
    Autonomia,
}

impl Canal {
    pub const TODOS: [Canal; 7] = [
        Canal::Rodas,
        Canal::Torre,
        Canal::Som,
        Canal::Energia,
        Canal::Modo,
        Canal::Ajustes,
        Canal::Autonomia,
    ];

    pub fn nome(self) -> &'static str {
        match self {
            Self::Rodas => "rodas",
            Self::Torre => "torre",
            Self::Som => "som",
            Self::Energia => "energia",
            Self::Modo => "modo",
            Self::Ajustes => "ajustes",
            Self::Autonomia => "autonomia",
        }
    }
}

/// Uma linha da tabela de quem manda no corpo. O desenho é o do `twist_mux` do ROS (fonte com prioridade e prazo), por
/// canal: entre as fontes com pedido vivo num canal, vale a de maior prioridade; sem pedido novo dentro do prazo, o canal
/// passa à seguinte. A parada (`robot.stop`) não passa por aqui: vale para qualquer fonte.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fonte {
    /// O nome que o cliente diz no `hello` (`client`).
    pub nome: String,
    /// De 0 a 255: maior manda. Os números só ordenam, e dois iguais são recusados.
    pub prioridade: u8,
    pub canais: Vec<Canal>,
    /// Sem pedido novo há este tempo, a fonte solta o canal. Ausente, vale `[seguranca] prazo_ms`.
    #[serde(default)]
    pub prazo_ms: Option<u64>,
    /// Só vale com a autonomia ligada (`robot.autonomous`): é a linha do cérebro.
    #[serde(default)]
    pub so_autonomo: bool,
}

impl Fonte {
    /// A tabela padrão, na ordem do dono (2026-11-19): piloto, agente, cérebro. Quem não diz o nome segue podendo o que
    /// podia antes da tabela, e perde para todos. É a mesma que `deploy/s1/rms1d.toml` escreve por extenso.
    pub fn padrao() -> Vec<Fonte> {
        let linha = |nome: &str, prioridade, canais: &[Canal], so_autonomo| Fonte {
            nome: nome.to_owned(),
            prioridade,
            canais: canais.to_vec(),
            prazo_ms: None,
            so_autonomo,
        };
        let corpo = [Canal::Rodas, Canal::Torre, Canal::Som];
        vec![
            linha("piloto", 30, &Canal::TODOS, false),
            linha("agente", 20, &corpo, false),
            linha("cerebro", 10, &corpo, true),
            linha(FONTE_SEM_NOME, 0, &Canal::TODOS, false),
        ]
    }
}

/// O robô por conta própria: o nível que `robot.autonomous` muda até o serviço sair, e com que ele sobe.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Autonomia {
    pub ligada: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Robo {
    pub nome: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoCorpo {
    Serial,
    Rede,
    Falso,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permissao {
    /// 900 para 306, o que a DJI envia (`biblioteca/guia/chassi.md`, `RM_FC_RMC`).
    Dji,
    /// Plano B, os bytes que moveram o robô em 2026-10-29.
    Proroklab,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Corpo {
    pub tipo: TipoCorpo,
    pub porta: String,
    /// Medido no robô com `stty` em 2026-10-30 (`scripts/s1/bench-selftest.sh`).
    pub baud: u32,
    /// Onde a ponte do simulador escuta quadros.
    pub endereco: String,
    /// Grava cada quadro enviado, para comparar simulador e robô. Vazio não grava.
    pub gravar: Option<PathBuf>,
    /// Grava cada leitura do corpo, `ms hex`, para reprocessar sem o robô. Exige `gravar`. Vazio não grava.
    pub gravar_recebido: Option<PathBuf>,
    /// O limite dos dois arquivos da gravação somados, em MiB: chegando nele, ela para, e o laço segue.
    pub gravar_limite_mb: u64,
    pub permissao: Permissao,
}

/// Números da DJI: centena o hospedeiro, resto o índice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Nos {
    pub remetente: u16,
    pub remetente_alavanca: u16,
    pub chassi: u16,
    pub torre: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Laco {
    /// A DJI envia a alavanca a cerca de 48 por segundo (`biblioteca/guia/chassi.md`).
    pub hz: u32,
    /// Abaixo disto o laço está doente.
    pub hz_minimo: f64,
    /// Sem ciclo há este tempo, o laço travou.
    pub idade_maxima_ms: u64,
    /// O controlador fala o tempo todo (medido em 2026-10-30): sem nenhum quadro por este tempo, a serial está muda.
    pub serial_muda_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Seguranca {
    /// O mesmo do pato (`duck-control/src/safety.rs`): sem comando novo, as rodas zeram.
    pub prazo_ms: u64,
    /// Quantas vezes o modo do chassi vai ao armar: a prova de 2026-10-29 enviou cinco.
    pub vezes_modo_ao_armar: u32,
    /// Comandos de parar ao desarmar, antes de cortar a permissão.
    pub vezes_zero_ao_desarmar: u32,
    /// Daqui para baixo, da tabela física.
    #[serde(skip)]
    pub velocidade_max_m_s: f64,
    #[serde(skip)]
    pub giro_max_grau_s: f64,
    #[serde(skip)]
    pub rpm_max: f64,
    #[serde(skip)]
    pub yaw_max_grau: f64,
    #[serde(skip)]
    pub pitch_min_grau: f64,
    #[serde(skip)]
    pub pitch_max_grau: f64,
    #[serde(skip)]
    pub giro_torre_max_grau_s: f64,
}

/// Da tabela física: `raio_roda` e `meia_soma`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometria {
    pub raio_roda_m: f64,
    pub meia_soma_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Torre {
    /// 4 é yaw no chassi e pitch NED, padrão de `angle_ctrl` (`rm_ctrl.py:1662`). Plano B: 3, que
    /// moveu a torre em 2026-10-29.
    pub coordenada: u8,
    /// Campo de velocidade de `0x3f/0xb0`, da tabela física.
    #[serde(skip)]
    pub velocidade: u16,
    /// Reenvio do alvo, em milissegundos, enquanto ele não muda.
    pub reenvio_ms: u64,
}

/// A assinatura da telemetria. O formato e o ritmo são os da DJI, e ficam em `rms1-proto`; aqui, só as esperas.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Telemetria {
    /// Quanto observar antes de cancelar: há mensagem de 1 Hz no ar (`s2.strace`), e ela tem de ser vista.
    pub janela_ms: u64,
    /// Quanto esperar por uma resposta do controlador. Só há 17 pares gravados, de 2 a 14 ms: isto é folga, e não medida.
    pub espera_ms: u64,
    pub tentativas: u32,
    /// Sem pacote há este tempo, a saúde diz que a telemetria parou. O ritmo é de 20 ms.
    pub idade_maxima_ms: u64,
    /// Sem pacote há este tempo, recomeça a assinatura.
    pub recomecar_parado_ms: u64,
    /// Depois de desistir por falta de resposta, quanto esperar para tentar de novo, se houver quadros chegando.
    pub recomecar_sem_resposta_ms: u64,
}

/// O que o `rms1d` calcula por conta própria, ao lado do controlador, em modo sombra. São limiares do cálculo, e não números
/// do robô: o físico (raio da roda, meia soma) vem da tabela. A origem de cada padrão está em `Default`.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Estimativa {
    /// A média da velocidade pelas rodas, em tempo do relógio do robô, e não em pacotes: num buraco da gravação, uma
    /// janela de 10 pacotes arrastava zeros de 600 ms antes e errava 0,21 m/s (medido em 2026-11-03, a 118 s do modo DJI).
    pub janela_ms: u64,
    /// Levantado: o controlador acusa mais velocidade que as rodas, por mais do que isto.
    pub levantado_limiar_m_s: f64,
    /// Por este tempo, seguido, para marcar levantado.
    pub levantado_tempo_ms: u64,
    /// Abaixo disto, por `soltar_tempo_ms`, deixa de marcar.
    pub soltar_limiar_m_s: f64,
    pub soltar_tempo_ms: u64,
    /// Quieto, para conferir a gravidade: giro abaixo disto, em cada eixo.
    pub quieto_giro_rad_s: f64,
    /// Quieto: o módulo da aceleração a menos disto de 1 g.
    pub quieto_aceleracao_g: f64,
    /// Acima disto, entre a inclinação pela atitude do controlador e a pela aceleração, o aviso.
    pub divergencia_grau: f64,
    /// Entre dois pacotes, acima disto não se integra: o relógio do robô recomeçou, ou a gravação perdeu dados.
    pub lacuna_ms: u64,
    /// Levantado também quando o giro pelas rodas passa o do giroscópio por mais disto, em rad/s, por `giro_levantado_tempo_ms`.
    pub giro_levantado_rad_s: f64,
    pub giro_levantado_tempo_ms: u64,
}

/// As placas de golpe.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Armadura {
    /// A sensibilidade da DJI, de 0 a 10: maior, golpe mais leve conta (`RobotArmor.set_hit_sensitivity`, `rm_ctrl.py`).
    pub sensibilidade: u8,
}

/// A mira da DJI: a posição em que o app a desenha, que o usuário ajusta e o robô guarda.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Mira {
    /// O arquivo em que o `dji_hdvt_uav` guarda as quatro posições (`/data/rm_user_setting.json`, visto de dentro do chroot).
    /// Sem ele, `s1.sight` responde o padrão de fábrica e recusa gravar.
    pub arquivo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Dji {
    /// Processos que seguram a serial (`scripts/s1/bench.sh:147`).
    pub parar: Vec<String>,
}

/// O serviço de mídia do S1: câmera, microfone e alto-falante, com um dono só (`rms1-mediad`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Midia {
    /// Onde o `rms1-mediad` serve `media.frame` e recebe `robot.sound`.
    pub socket: String,
    pub camera: MidiaCamera,
    pub microfone: MidiaMicrofone,
    pub alto_falante: MidiaAltoFalante,
    /// Depois de o robô tocar um som, por quanto tempo o que o microfone ouve é descartado.
    pub trava_ms: u64,
    pub percepcao: MidiaPercepcao,
    /// O trecho de voz que o serviço guarda do microfone e manda à estação.
    pub voz: MidiaVoz,
    /// Se o serviço grava o mixer do chip de áudio ao subir, como a partida da DJI grava. Só no robô, com a DJI parada.
    pub mixer: bool,
    /// A porta TCP em que o serviço entrega o quadro reduzido a quem só quer olhar (a tela do simulador), direto pela
    /// rede. Zero: não há porta. Por ela não passa nada que mude o robô.
    pub porta_video: u16,
    /// O socket do `rms1-venc`, o programa do lado Android do robô que comanda o codificador de hardware (H.264 e JPEG).
    /// Vazio: não há codificador, e o quadro vai cru à estação e a imagem da tela é codificada por software.
    pub venc: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FonteCamera {
    /// Um padrão que se move, de 256 por 144: para máquina sem câmera.
    Teste,
    /// A câmera do robô simulado: a página desenha o quadro, 1280 por 720, só quando alguém o pede.
    Simulador,
    /// A câmera do robô, `/dev/video0`, aberta pela sequência da DJI. Só no robô, com o `dji_camera` parado.
    S1,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MidiaCamera {
    pub fonte: FonteCamera,
    /// Onde o serviço local do simulador entrega a câmera simulada (`endereço:porta`).
    pub endereco: String,
    /// O dispositivo da câmera do robô.
    pub dispositivo: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FonteMicrofone {
    /// Sem microfone: nada é ouvido.
    Nenhuma,
    /// Um `.wav`, tocado em laço no ritmo real.
    Arquivo,
    /// O microfone do robô, por um `arecord` no dispositivo dado.
    Arecord,
    /// O microfone do robô simulado: o som que a página manda pelo serviço local (PCM cru, S16LE, mono, 16 kHz).
    Simulador,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MidiaMicrofone {
    pub fonte: FonteMicrofone,
    /// O `.wav` da fonte de arquivo.
    pub arquivo: Option<String>,
    /// O dispositivo do `arecord`: o `micleft` de `deploy/s1/asound.conf`, o canal esquerdo a 16 kHz.
    pub dispositivo: String,
    /// Onde o serviço local do simulador entrega o som do microfone simulado (`endereço:porta`).
    pub endereco: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaidaDeSom {
    /// Um `aplay` por som, no dispositivo dado.
    Aplay,
    /// Nada toca.
    Nula,
    /// O alto-falante do robô simulado: o som vai à página, pelo serviço local, e ela o toca.
    Simulador,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MidiaAltoFalante {
    pub saida: SaidaDeSom,
    pub dispositivo: String,
    /// Onde fica o banco de voz deste robô (`sounds ensure-bank`).
    pub banco: String,
    /// Onde o serviço local do simulador recebe o som do alto-falante simulado (`endereço:porta`).
    pub endereco: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MidiaPercepcao {
    /// A porta UDP em que a estação Hailo se anuncia.
    pub porta_anuncio: u16,
    /// O teto de quadros por segundo que o S1 manda à estação, seja qual for o ritmo anunciado.
    pub ritmo_max: u32,
}

/// Como o serviço corta do microfone o trecho de voz que vai à estação. O nível é medido a cada 32 ms, contra um piso que
/// acompanha a sala fora da fala. Os dois múltiplos são os do vigia de som do pato (`pet-detect/src/worker.rs`), medidos no
/// microfone dele; os do S1 se medem no robô.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MidiaVoz {
    /// Quanto do som de antes do começo da fala vai junto, para a primeira sílaba não ser cortada.
    pub folga_ms: u64,
    /// Quanto de silêncio encerra o trecho.
    pub silencio_ms: u64,
    /// O maior trecho: o som que não para é cortado aqui. O contrato da estação aceita até 10 s.
    pub teto_ms: u64,
    /// O menor som alto que conta como fala: abaixo disto é estalo ou palma, e não vai.
    pub minimo_ms: u64,
    /// Quantas vezes o piso da sala o nível tem de passar para a fala começar.
    pub entra: f32,
    /// Abaixo de quantas vezes o piso o nível conta como silêncio.
    pub sai: f32,
}

impl Default for MidiaVoz {
    fn default() -> Self {
        Self {
            folga_ms: 300,
            // Pedido do dono em 2026-11-18, ao falar com o robô: a pausa no meio da frase a cortava em duas. Com 1,2 s a
            // pausa comum cabe; o custo é a resposta sair meio segundo mais tarde.
            silencio_ms: 1200,
            teto_ms: 10_000,
            // O vigia do pato chama de estalo o som de até 0,38 s (a palma e o eco dela).
            minimo_ms: 400,
            // Medido no S1 em 2026-11-18, com o ar-condicionado ligado: o silêncio em 0,0023 e a fala normal a meio metro em
            // 0,009 a 0,023, de quatro a dez vezes o silêncio. Com 6 e 3, a maior parte da fala ficava de fora.
            entra: 3.0,
            sai: 2.0,
        }
    }
}

impl Default for Midia {
    fn default() -> Self {
        Self {
            socket: "/run/rms1-mediad/media.sock".to_owned(),
            camera: MidiaCamera::default(),
            microfone: MidiaMicrofone::default(),
            alto_falante: MidiaAltoFalante::default(),
            // A janela do runtime antigo do pato (`microduck_runtime/src/main.rs:2921-2935`). A do S1 se mede no robô.
            trava_ms: 800,
            percepcao: MidiaPercepcao::default(),
            voz: MidiaVoz::default(),
            mixer: false,
            porta_video: 0,
            venc: String::new(),
        }
    }
}

impl Default for MidiaCamera {
    fn default() -> Self {
        Self {
            fonte: FonteCamera::Teste,
            // A porta da câmera no serviço local do simulador (`tools/simulador/s1_servico.py`, `--camera`, no laboratório).
            endereco: "127.0.0.1:9210".to_owned(),
            dispositivo: "/dev/video0".to_owned(),
        }
    }
}

impl Default for MidiaMicrofone {
    fn default() -> Self {
        Self {
            fonte: FonteMicrofone::Nenhuma,
            arquivo: None,
            dispositivo: "micleft".to_owned(),
            // A porta do som no serviço local do simulador (`tools/simulador/s1_servico.py`, `--som`, no laboratório).
            endereco: "127.0.0.1:9211".to_owned(),
        }
    }
}

impl Default for MidiaAltoFalante {
    fn default() -> Self {
        Self {
            saida: SaidaDeSom::Nula,
            // O que tocou a voz do pato no S1 em 2026-10-31 (`docs/s1/project/s1-bringup.md:1368-1373`).
            dispositivo: "plughw:0,0".to_owned(),
            // O lugar do banco no pato (`robotd-params`, `bank`).
            banco: "/var/lib/robot/sounds".to_owned(),
            endereco: "127.0.0.1:9211".to_owned(),
        }
    }
}

impl Default for MidiaPercepcao {
    fn default() -> Self {
        Self {
            porta_anuncio: 45679,
            // Um quadro por vez nos 45,37 ms de ida e volta medidos em 2026-11-13 (S1, Pi, S1).
            ritmo_max: 22,
        }
    }
}

impl Default for Robo {
    fn default() -> Self {
        Self {
            nome: "rms1".to_owned(),
        }
    }
}

impl Default for Corpo {
    fn default() -> Self {
        Self {
            tipo: TipoCorpo::Falso,
            porta: "/dev/ttyS3".to_owned(),
            baud: 921_600,
            endereco: "127.0.0.1:9100".to_owned(),
            gravar: None,
            gravar_recebido: None,
            // Cerca de 27 minutos: o robô grava 2,49 MB por minuto (2.273.280 bytes em 54,8 s, medido em 2026-11-15, na
            // gravação `robo-140456`). Sem limite, duas gravações esquecidas encheram o disco do robô nesse dia.
            gravar_limite_mb: 64,
            permissao: Permissao::Dji,
        }
    }
}

impl Default for Nos {
    fn default() -> Self {
        Self {
            remetente: 906,
            remetente_alavanca: 900,
            chassi: 306,
            torre: 400,
        }
    }
}

impl Default for Laco {
    fn default() -> Self {
        Self {
            // O ritmo da DJI: `RmStickLoopTimer`, 10000 µs, em `dji_hdvt_uav`; 10,1 ms de intervalo mediano na gravação `seguir`.
            hz: 100,
            hz_minimo: 80.0,
            idade_maxima_ms: 500,
            serial_muda_ms: 3000,
        }
    }
}

impl Default for Seguranca {
    fn default() -> Self {
        Self {
            prazo_ms: 500,
            vezes_modo_ao_armar: 5,
            vezes_zero_ao_desarmar: 10,
            ..Fisica::da_tabela().seguranca
        }
    }
}

impl Default for Geometria {
    fn default() -> Self {
        Fisica::da_tabela().geometria
    }
}

impl Default for Torre {
    fn default() -> Self {
        Self {
            coordenada: 4,
            velocidade: Fisica::da_tabela().velocidade_torre,
            reenvio_ms: 100,
        }
    }
}

impl Default for Telemetria {
    fn default() -> Self {
        Self {
            janela_ms: 2500,
            espera_ms: 500,
            tentativas: 3,
            idade_maxima_ms: 500,
            recomecar_parado_ms: 3000,
            recomecar_sem_resposta_ms: 10_000,
        }
    }
}

impl Default for Estimativa {
    fn default() -> Self {
        Self {
            // As gravações de 2026-11-03 (`out/2026-11-03-firmware-controlador/`, no laboratório), reprocessadas com
            // `rms1d-reprocessar`: os números abaixo são os que passam nelas, e o teste `estimativa_lab` os prova.
            // 10 pacotes a 50 por segundo.
            janela_ms: 200,
            // O mecanismo e os números de partida são os do `s1d` (`s1d/src/tick.rs`, `LIFT_TRIP_M_S`), que os diz sem
            // origem; conferidos nas gravações de 2026-11-03: disparam nos manuseios e não andando.
            levantado_limiar_m_s: 0.4,
            // Medido em 2026-11-03: freando e arrancando a até 2,9 m/s, a discordância passou de 0,4 m/s por 0,28 a 1,10 s
            // (as rodas param antes do robô); nos manuseios, o primeiro trecho durou de 2,94 a 16,38 s. 300 ms, o do `s1d`,
            // marcava levantado nas freadas (`out/2026-11-03-firmware-controlador/sombra/`, no laboratório).
            levantado_tempo_ms: 2000,
            soltar_limiar_m_s: 0.15,
            soltar_tempo_ms: 500,
            // Com estes limites e o bit de parado, 3831 e 6569 pacotes passam como quietos nas duas gravações de 2026-11-03
            // (`rms1d-reprocessar`, linha "gravidade, quieto").
            quieto_giro_rad_s: 0.1,
            quieto_aceleracao_g: 0.03,
            // Quietos, a atitude e a aceleração diferem até 3,0° nessas gravações; a atitude presa da imagem 60 diferia de 24°.
            divergencia_grau: 10.0,
            // O ritmo é de 20 ms (`gravacoes_lab.rs`, `o_relogio_anda_20_ms_por_pacote`): 1 s são 50 pacotes perdidos.
            lacuna_ms: 1000,
            // Medido em 2026-11-03, giro pelas rodas menos o do giroscópio: no chão, andando e seguindo a torre, acima de 3 rad/s
            // por no máximo 0,93 s seguidos (gravação em sombra) e 0,04 s (gravação das 20:40); com o robô no ar e o controlador
            // alinhando o chassi, 3,84 s seguidos. 1,5 s fica acima do pior caso no chão (`out/2026-11-03-firmware-controlador/`).
            giro_levantado_rad_s: 3.0,
            giro_levantado_tempo_ms: 1500,
        }
    }
}

impl Default for Armadura {
    fn default() -> Self {
        // O padrão da DJI: `reset_hit_sensitivity` volta a 5, e 5 é o que a central mandou ao ligar (`bancada-150s`).
        Self { sensibilidade: 5 }
    }
}

impl Default for Dji {
    fn default() -> Self {
        Self {
            // O monitor primeiro: ele religa quem cair.
            parar: vec!["dji_monitor".to_owned(), "dji_hdvt_uav".to_owned()],
        }
    }
}

/// O que o `rms1d` lê da tabela física.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fisica {
    pub seguranca: Seguranca,
    pub geometria: Geometria,
    pub velocidade_torre: u16,
}

impl Fisica {
    /// A tabela é embutida e testada: um erro aqui é erro de compilação do projeto, não de quem roda.
    pub fn da_tabela() -> Self {
        Self::de_json(TABELA).expect("robo_s1.json inválido")
    }

    /// Qualquer tabela, para os testes provarem que o número vem daqui.
    pub fn de_json(texto: &str) -> Result<Self, String> {
        let t: serde_json::Value = serde_json::from_str(texto).map_err(|e| e.to_string())?;
        let n = |chave: &str, campo: Option<&str>| -> Result<f64, String> {
            let v = &t[chave]["valor"];
            campo
                .map_or(v, |c| &v[c])
                .as_f64()
                .ok_or_else(|| format!("tabela física sem {chave}"))
        };
        Ok(Self {
            seguranca: Seguranca {
                prazo_ms: 0,
                vezes_modo_ao_armar: 0,
                vezes_zero_ao_desarmar: 0,
                velocidade_max_m_s: n("velocidades_chassi", Some("frente"))?,
                giro_max_grau_s: n("giro_chassi", None)?,
                rpm_max: n("rpm_roda", None)?,
                yaw_max_grau: n("limites_torre", Some("yaw"))?,
                pitch_min_grau: n("limites_torre", Some("pitchMin"))?,
                pitch_max_grau: n("limites_torre", Some("pitchMax"))?,
                giro_torre_max_grau_s: n("giro_torre", None)?,
            },
            geometria: Geometria {
                raio_roda_m: n("raio_roda", None)?,
                meia_soma_m: n("meia_soma", None)?,
            },
            velocidade_torre: n("velocidade_torre", None)? as u16,
        })
    }
}

impl Params {
    /// Estes parâmetros com outra tabela física.
    pub fn com_tabela(mut self, texto: &str) -> Result<Self, String> {
        let f = Fisica::de_json(texto)?;
        self.seguranca = Seguranca {
            prazo_ms: self.seguranca.prazo_ms,
            vezes_modo_ao_armar: self.seguranca.vezes_modo_ao_armar,
            vezes_zero_ao_desarmar: self.seguranca.vezes_zero_ao_desarmar,
            ..f.seguranca
        };
        self.geometria = f.geometria;
        self.torre.velocidade = f.velocidade_torre;
        Ok(self)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParamsErro {
    #[error("não consegui ler {caminho}: {causa}")]
    Leitura {
        caminho: PathBuf,
        causa: std::io::Error,
    },
    #[error("{caminho} é inválido: {causa}")]
    Toml {
        caminho: PathBuf,
        causa: Box<toml::de::Error>,
    },
    #[error("{0}")]
    Valor(String),
}

impl Params {
    /// `explicito` é o caminho dado na linha de comando: esse tem de existir.
    pub fn carregar(caminho: &Path, explicito: bool) -> Result<Self, ParamsErro> {
        let texto = match std::fs::read_to_string(caminho) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicito => {
                return Ok(Self::default());
            }
            Err(causa) => {
                return Err(ParamsErro::Leitura {
                    caminho: caminho.to_owned(),
                    causa,
                });
            }
        };
        let p: Self = toml::from_str(&texto).map_err(|causa| ParamsErro::Toml {
            caminho: caminho.to_owned(),
            causa: Box::new(causa),
        })?;
        p.validar()?;
        Ok(p)
    }

    pub fn validar(&self) -> Result<(), ParamsErro> {
        let ruim = |m: String| Err(ParamsErro::Valor(m));
        if !(1..=1000).contains(&self.laco.hz) {
            return ruim(format!("laco.hz fora de 1 a 1000: {}", self.laco.hz));
        }
        if self.seguranca.prazo_ms == 0 {
            return ruim("seguranca.prazo_ms tem de ser maior que zero".to_owned());
        }
        if self.seguranca.vezes_modo_ao_armar == 0 || self.seguranca.vezes_zero_ao_desarmar == 0 {
            return ruim(
                "seguranca: as repetições ao armar e ao desarmar têm de ser ao menos 1".to_owned(),
            );
        }
        let t = &self.telemetria;
        if t.espera_ms == 0 || t.tentativas == 0 || t.janela_ms < 1100 {
            return ruim(format!(
                "telemetria: espera e tentativas maiores que zero, e janela de ao menos 1100 ms, para ver a mensagem de 1 Hz: {t:?}"
            ));
        }
        if self.corpo.gravar_recebido.is_some() && self.corpo.gravar.is_none() {
            return ruim(
                "corpo.gravar_recebido precisa de corpo.gravar: as duas gravações andam juntas"
                    .to_owned(),
            );
        }
        // Zero desligaria a gravação sem dizer; acima disto, uma gravação sozinha ocupa mais da metade do disco do robô.
        if !(1..=512).contains(&self.corpo.gravar_limite_mb) {
            return ruim(format!(
                "corpo.gravar_limite_mb fora de 1 a 512: {}",
                self.corpo.gravar_limite_mb
            ));
        }
        let e = &self.estimativa;
        if !(1..=5000).contains(&e.janela_ms) {
            return ruim(format!(
                "estimativa.janela_ms fora de 1 a 5000: {}",
                e.janela_ms
            ));
        }
        for (nome, v, max) in [
            (
                "estimativa.levantado_limiar_m_s",
                e.levantado_limiar_m_s,
                10.0,
            ),
            ("estimativa.soltar_limiar_m_s", e.soltar_limiar_m_s, 10.0),
            ("estimativa.quieto_giro_rad_s", e.quieto_giro_rad_s, 10.0),
            ("estimativa.quieto_aceleracao_g", e.quieto_aceleracao_g, 1.0),
            ("estimativa.divergencia_grau", e.divergencia_grau, 90.0),
            (
                "estimativa.giro_levantado_rad_s",
                e.giro_levantado_rad_s,
                30.0,
            ),
        ] {
            if !(v.is_finite() && v > 0.0 && v <= max) {
                return ruim(format!("{nome} tem de estar entre 0 e {max}: {v}"));
            }
        }
        if e.soltar_limiar_m_s > e.levantado_limiar_m_s {
            return ruim(format!(
                "estimativa.soltar_limiar_m_s ({}) acima de levantado_limiar_m_s ({}): marcaria e soltaria no mesmo valor",
                e.soltar_limiar_m_s, e.levantado_limiar_m_s
            ));
        }
        // Um tempo absurdo desliga o cálculo em silêncio: o levantado nunca marcaria, ou todo pacote seria lacuna.
        for (nome, v, min, max) in [
            (
                "estimativa.levantado_tempo_ms",
                e.levantado_tempo_ms,
                1,
                10_000,
            ),
            ("estimativa.soltar_tempo_ms", e.soltar_tempo_ms, 1, 10_000),
            (
                "estimativa.giro_levantado_tempo_ms",
                e.giro_levantado_tempo_ms,
                1,
                10_000,
            ),
            // Abaixo de 100 ms, cinco pacotes a 50 por segundo, o ritmo normal já seria lacuna.
            ("estimativa.lacuna_ms", e.lacuna_ms, 100, 60_000),
        ] {
            if !(min..=max).contains(&v) {
                return ruim(format!("{nome} fora de {min} a {max}: {v}"));
            }
        }
        for (nome, v) in [
            ("geometria.raio_roda_m", self.geometria.raio_roda_m),
            ("geometria.meia_soma_m", self.geometria.meia_soma_m),
            (
                "seguranca.velocidade_max_m_s",
                self.seguranca.velocidade_max_m_s,
            ),
            ("seguranca.giro_max_grau_s", self.seguranca.giro_max_grau_s),
            ("seguranca.rpm_max", self.seguranca.rpm_max),
            ("seguranca.yaw_max_grau", self.seguranca.yaw_max_grau),
            (
                "seguranca.giro_torre_max_grau_s",
                self.seguranca.giro_torre_max_grau_s,
            ),
        ] {
            if !(v.is_finite() && v > 0.0) {
                return ruim(format!("{nome} tem de ser positivo: {v}"));
            }
        }
        if self.seguranca.pitch_min_grau >= self.seguranca.pitch_max_grau {
            return ruim("seguranca.pitch_min_grau tem de ser menor que pitch_max_grau".to_owned());
        }
        if let Some(a) = &self.mira.arquivo
            && !a.starts_with('/')
        {
            return ruim(format!("mira.arquivo tem de ser um caminho absoluto: {a}"));
        }
        if self.armadura.sensibilidade > 10 {
            return ruim(format!(
                "armadura.sensibilidade fora de 0 a 10, a faixa da DJI: {}",
                self.armadura.sensibilidade
            ));
        }
        if self.torre.coordenada > 7 {
            return ruim(format!(
                "torre.coordenada cabe em 3 bits: {}",
                self.torre.coordenada
            ));
        }
        if let Some(p) = self
            .dji
            .parar
            .iter()
            .find(|p| !PODE_PARAR.contains(&p.as_str()))
        {
            return ruim(format!(
                "dji.parar não pode ter `{p}`: só {PODE_PARAR:?} podem parar"
            ));
        }
        if self.fontes.len() > FONTES_MAX {
            return ruim(format!(
                "fontes: a tabela tem {} linhas, e o teto é {FONTES_MAX}",
                self.fontes.len()
            ));
        }
        if !self.fontes.iter().any(|f| f.nome == FONTE_SEM_NOME) {
            return ruim(format!(
                "fontes: falta a linha `{FONTE_SEM_NOME}`, a de quem não diz o nome no hello"
            ));
        }
        for (i, f) in self.fontes.iter().enumerate() {
            if f.nome.is_empty() {
                return ruim(format!("fontes: a linha {} está sem nome", i + 1));
            }
            if self.fontes[..i].iter().any(|g| g.nome == f.nome) {
                return ruim(format!("fontes: o nome `{}` está repetido", f.nome));
            }
            if let Some(g) = self.fontes[..i]
                .iter()
                .find(|g| g.prioridade == f.prioridade)
            {
                return ruim(format!(
                    "fontes: `{}` e `{}` têm a mesma prioridade ({}), e a ordem entre elas ficaria por conta do acaso",
                    g.nome, f.nome, f.prioridade
                ));
            }
            // Zero soltaria o canal antes do ciclo seguinte; acima de 10 s, um cliente caído seguraria o corpo.
            if let Some(p) = f.prazo_ms
                && !(1..=10_000).contains(&p)
            {
                return ruim(format!(
                    "fontes: prazo_ms de `{}` fora de 1 a 10000: {p}",
                    f.nome
                ));
            }
        }
        if let Some(m) = &self.midia {
            if !m.socket.starts_with('/') {
                return ruim(format!(
                    "midia.socket tem de ser um caminho absoluto: {}",
                    m.socket
                ));
            }
            // Uma janela absurda deixaria o robô surdo sem ninguém saber.
            if m.trava_ms > 10_000 {
                return ruim(format!("midia.trava_ms acima de 10000: {}", m.trava_ms));
            }
            if !(1..=22).contains(&m.percepcao.ritmo_max) {
                return ruim(format!(
                    "midia.percepcao.ritmo_max fora de 1 a 22: {}",
                    m.percepcao.ritmo_max
                ));
            }
            if m.percepcao.porta_anuncio == 0 {
                return ruim("midia.percepcao.porta_anuncio não pode ser 0".to_owned());
            }
            let v = &m.voz;
            if v.folga_ms > 2000 {
                return ruim(format!("midia.voz.folga_ms acima de 2000: {}", v.folga_ms));
            }
            if !(200..=3000).contains(&v.silencio_ms) {
                return ruim(format!(
                    "midia.voz.silencio_ms fora de 200 a 3000: {}",
                    v.silencio_ms
                ));
            }
            // O teto é o do contrato da estação: trecho de voz de até 10 s.
            if !(1000..=10_000).contains(&v.teto_ms) {
                return ruim(format!(
                    "midia.voz.teto_ms fora de 1000 a 10000: {}",
                    v.teto_ms
                ));
            }
            if v.minimo_ms >= v.teto_ms {
                return ruim(format!(
                    "midia.voz.minimo_ms tem de ficar abaixo do teto ({}): {}",
                    v.teto_ms, v.minimo_ms
                ));
            }
            if !(v.sai.is_finite() && v.sai >= 1.0) {
                return ruim(format!("midia.voz.sai tem de ser 1 ou mais: {}", v.sai));
            }
            if !(v.entra.is_finite() && v.entra > v.sai) {
                return ruim(format!(
                    "midia.voz.entra tem de ficar acima de sai ({}): {}",
                    v.sai, v.entra
                ));
            }
            if m.camera.fonte == FonteCamera::Simulador
                && m.camera.endereco.parse::<std::net::SocketAddr>().is_err()
            {
                return ruim(format!(
                    "midia.camera.endereco tem de ser endereço:porta: {}",
                    m.camera.endereco
                ));
            }
            if m.microfone.fonte == FonteMicrofone::Simulador
                && m.microfone
                    .endereco
                    .parse::<std::net::SocketAddr>()
                    .is_err()
            {
                return ruim(format!(
                    "midia.microfone.endereco tem de ser endereço:porta: {}",
                    m.microfone.endereco
                ));
            }
            if m.alto_falante.saida == SaidaDeSom::Simulador
                && m.alto_falante
                    .endereco
                    .parse::<std::net::SocketAddr>()
                    .is_err()
            {
                return ruim(format!(
                    "midia.alto_falante.endereco tem de ser endereço:porta: {}",
                    m.alto_falante.endereco
                ));
            }
            if m.microfone.fonte == FonteMicrofone::Arquivo && m.microfone.arquivo.is_none() {
                return ruim(
                    "midia.microfone.arquivo falta: a fonte de arquivo precisa do `.wav`"
                        .to_owned(),
                );
            }
            if !m.alto_falante.banco.starts_with('/') {
                return ruim(format!(
                    "midia.alto_falante.banco tem de ser um caminho absoluto: {}",
                    m.alto_falante.banco
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn de(texto: &str) -> Result<Params, ParamsErro> {
        let d = tempfile::tempdir().unwrap();
        let c = d.path().join("rms1d.toml");
        std::fs::write(&c, texto).unwrap();
        Params::carregar(&c, true)
    }

    #[test]
    fn arquivo_ausente_usa_os_padroes_e_caminho_explicito_tem_de_existir() {
        let d = tempfile::tempdir().unwrap();
        let c = d.path().join("nao-existe.toml");
        assert_eq!(Params::carregar(&c, false).unwrap(), Params::default());
        assert!(Params::carregar(&c, true).is_err());
    }

    #[test]
    fn um_valor_no_arquivo_muda_o_parametro() {
        let p = de("[seguranca]\nprazo_ms = 250\n[corpo]\ntipo = \"rede\"\n").unwrap();
        assert_eq!(p.seguranca.prazo_ms, 250);
        assert_eq!(p.corpo.tipo, TipoCorpo::Rede);
        assert_eq!(p.laco.hz, 100, "o resto segue o padrão");
    }

    /// Um prazo escrito errado deixaria o robô no padrão sem ninguém saber.
    #[test]
    fn chave_desconhecida_e_recusada_pelo_nome() {
        let e = de("[seguranca]\nprazzo_ms = 250\n")
            .unwrap_err()
            .to_string();
        assert!(e.contains("prazzo_ms"), "{e}");
    }

    #[test]
    fn valores_absurdos_sao_recusados() {
        for t in [
            "[laco]\nhz = 0\n",
            "[seguranca]\nprazo_ms = 0\n",
            "[torre]\ncoordenada = 9\n",
            "[seguranca]\nvezes_modo_ao_armar = 0\n",
            "[telemetria]\njanela_ms = 500\n",
            "[telemetria]\ntentativas = 0\n",
            "[armadura]\nsensibilidade = 11\n",
            "[mira]\narquivo = \"rm_user_setting.json\"\n",
        ] {
            assert!(de(t).is_err(), "{t}");
        }
    }

    /// Cada limiar absurdo é recusado pelo nome dele.
    #[test]
    fn limiar_absurdo_da_estimativa_e_recusado_pelo_nome() {
        for (t, nome) in [
            (
                "[estimativa]\ngiro_levantado_rad_s = 0.0\n",
                "giro_levantado_rad_s",
            ),
            (
                "[estimativa]\ngiro_levantado_tempo_ms = 0\n",
                "giro_levantado_tempo_ms",
            ),
            ("[estimativa]\njanela_ms = 0\n", "janela_ms"),
            ("[estimativa]\njanela_ms = 60000\n", "janela_ms"),
            ("[estimativa]\njanela_pacotes = 10\n", "janela_pacotes"),
            (
                "[estimativa]\nlevantado_limiar_m_s = 0.0\n",
                "levantado_limiar_m_s",
            ),
            (
                "[estimativa]\nlevantado_limiar_m_s = -1.0\n",
                "levantado_limiar_m_s",
            ),
            (
                "[estimativa]\nsoltar_limiar_m_s = 0.9\n",
                "soltar_limiar_m_s",
            ),
            (
                "[estimativa]\nquieto_aceleracao_g = 2.0\n",
                "quieto_aceleracao_g",
            ),
            (
                "[estimativa]\ndivergencia_grau = 180.0\n",
                "divergencia_grau",
            ),
            (
                "[estimativa]\nquieto_giro_rad_s = nan\n",
                "quieto_giro_rad_s",
            ),
            (
                "[estimativa]\nlevantado_tempo_ms = 0\n",
                "levantado_tempo_ms",
            ),
            (
                "[estimativa]\nlevantado_tempo_ms = 1000000000\n",
                "levantado_tempo_ms",
            ),
            ("[estimativa]\nsoltar_tempo_ms = 99999\n", "soltar_tempo_ms"),
            ("[estimativa]\nlacuna_ms = 0\n", "lacuna_ms"),
            ("[estimativa]\nlacuna_ms = 5\n", "lacuna_ms"),
            ("[estimativa]\nlimiar = 0.4\n", "limiar"),
            ("[corpo]\ngravar_recebido = \"/tmp/r\"\n", "gravar_recebido"),
        ] {
            let e = de(t).unwrap_err().to_string();
            assert!(e.contains(nome), "{t}: {e}");
        }
        let p = de("[estimativa]\njanela_ms = 100\n[corpo]\ngravar = \"/tmp/a\"\ngravar_recebido = \"/tmp/b\"\n").unwrap();
        assert_eq!(p.estimativa.janela_ms, 100);
        assert_eq!(
            p.estimativa.levantado_limiar_m_s, 0.4,
            "o resto segue o padrão"
        );
    }

    /// Adendo 2, critério 78: o limite da gravação vale 64 sem a chave, e fora de 1 a 512 é recusado pelo nome.
    #[test]
    fn limite_da_gravacao_e_recusado_pelo_nome() {
        assert_eq!(Params::default().corpo.gravar_limite_mb, 64);
        assert_eq!(
            de("[corpo]\ngravar_limite_mb = 8\n")
                .unwrap()
                .corpo
                .gravar_limite_mb,
            8
        );
        for t in [
            "[corpo]\ngravar_limite_mb = 0\n",
            "[corpo]\ngravar_limite_mb = 513\n",
        ] {
            let e = de(t).unwrap_err().to_string();
            assert!(e.contains("gravar_limite_mb"), "{t}: {e}");
        }
    }

    /// O S1 é um só: número físico no arquivo é recusado pelo nome.
    #[test]
    fn numero_fisico_em_rms1d_toml_e_recusado_pelo_nome() {
        for (t, nome) in [
            ("[geometria]\nraio_roda_m = 0.06\n", "geometria"),
            ("[seguranca]\nrpm_max = 500.0\n", "rpm_max"),
            (
                "[seguranca]\nvelocidade_max_m_s = 1.0\n",
                "velocidade_max_m_s",
            ),
            ("[torre]\nvelocidade = 30\n", "velocidade"),
        ] {
            let e = de(t).unwrap_err().to_string();
            assert!(e.contains(nome), "{t}: {e}");
        }
    }

    /// Cada entrada diz de onde veio: `parametros_s1.js` pedia isso, e continua valendo.
    #[test]
    fn toda_entrada_da_tabela_tem_valor_unidade_origem_e_prova() {
        let t: serde_json::Value = serde_json::from_str(TABELA).unwrap();
        let t = t.as_object().unwrap();
        assert!(t.len() > 40, "{}", t.len());
        for (chave, e) in t {
            assert!(!e["valor"].is_null(), "{chave}: sem valor");
            assert!(e["unidade"].is_string(), "{chave}: sem unidade");
            let origem = e["origem"].as_str().unwrap_or_default();
            assert!(
                ["pesado", "dji", "calculado", "constante", "provisorio"].contains(&origem),
                "{chave}: origem {origem}"
            );
            let prova = if origem == "provisorio" {
                "como_verificar"
            } else {
                "prova"
            };
            assert!(e[prova].is_string(), "{chave}: sem {prova}");
        }
    }

    #[test]
    fn os_padroes_sao_os_da_tabela() {
        let p = Params::default();
        assert_eq!(p.seguranca.rpm_max, 1000.0);
        assert_eq!(p.geometria.meia_soma_m, 0.1949);
        assert_eq!(p.torre.velocidade, 120);
        assert_eq!(p.seguranca.prazo_ms, 500, "o prazo não é físico");
    }

    /// Número físico do robô só na tabela. Por texto, de leve, no código que roda, sem os testes.
    #[test]
    fn nenhum_numero_fisico_fora_da_tabela() {
        let raiz = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
        let mut arquivos = Vec::new();
        for pasta in [
            "rms1-control/src",
            "rms1d/src",
            "rms1d/src/bin",
            "rms1d-params/src",
            "rms1-mediad/src",
        ] {
            for a in std::fs::read_dir(format!("{raiz}/{pasta}"))
                .unwrap()
                .flatten()
            {
                if a.path().extension().is_some_and(|x| x == "rs") {
                    arquivos.push(a.path());
                }
            }
        }
        assert!(arquivos.len() > 8, "{arquivos:?}");
        let mut achados = Vec::new();
        for a in arquivos {
            let texto = std::fs::read_to_string(&a).unwrap();
            let codigo = texto.split("#[cfg(test)]").next().unwrap();
            // 1000.0, o limite de rpm, fica de fora: `saude.rs` usa o mesmo número para milésimos de hertz.
            for n in [
                "0.195", "0.1949", "0.05", "540.0", "600.0", "250.0", "35.0", "3.5",
            ] {
                if codigo
                    .lines()
                    .any(|l| !l.trim_start().starts_with("//") && l.contains(n))
                {
                    achados.push(format!("{}: {n}", a.display()));
                }
            }
        }
        assert!(achados.is_empty(), "{achados:?}");
    }

    /// Sem a seção, não há serviço de mídia; com ela vazia, valem os padrões; e cada erro é recusado pelo nome.
    #[test]
    fn midia_errada_e_recusada_pelo_nome() {
        assert_eq!(Params::default().midia, None);
        let p = de("[midia]\n").unwrap().midia.unwrap();
        assert_eq!(p, Midia::default());
        assert_eq!(
            (p.trava_ms, p.percepcao.porta_anuncio, p.percepcao.ritmo_max),
            (800, 45679, 22)
        );
        let p = de("[midia]\ntrava_ms = 400\n[midia.microfone]\nfonte = \"arquivo\"\narquivo = \"/tmp/a.wav\"\n[midia.alto_falante]\nsaida = \"aplay\"\n").unwrap().midia.unwrap();
        assert_eq!(
            (p.trava_ms, p.microfone.fonte, p.alto_falante.saida),
            (400, FonteMicrofone::Arquivo, SaidaDeSom::Aplay)
        );
        // A voz que vai à estação: os padrões, e cada valor fora da faixa recusado pelo nome.
        assert_eq!(
            (
                p.voz.folga_ms,
                p.voz.silencio_ms,
                p.voz.teto_ms,
                p.voz.minimo_ms,
                p.voz.entra,
                p.voz.sai
            ),
            (300, 1200, 10_000, 400, 3.0, 2.0)
        );
        let p = de("[midia.voz]\nsilencio_ms = 500\nteto_ms = 8000\n")
            .unwrap()
            .midia
            .unwrap();
        assert_eq!(
            (p.voz.silencio_ms, p.voz.teto_ms, p.voz.folga_ms),
            (500, 8000, 300)
        );
        for (t, nome) in [
            ("[midia]\ntrava = 800\n", "trava"),
            ("[midia]\ntrava_ms = 10001\n", "trava_ms"),
            ("[midia]\nsocket = \"media.sock\"\n", "midia.socket"),
            ("[midia.percepcao]\nritmo_max = 0\n", "ritmo_max"),
            ("[midia.percepcao]\nritmo_max = 23\n", "ritmo_max"),
            ("[midia.percepcao]\nporta_anuncio = 0\n", "porta_anuncio"),
            ("[midia.camera]\nfonte = \"webcam\"\n", "webcam"),
            (
                "[midia.camera]\nfonte = \"simulador\"\nendereco = \"aqui\"\n",
                "midia.camera.endereco",
            ),
            (
                "[midia.microfone]\nfonte = \"arquivo\"\n",
                "midia.microfone.arquivo",
            ),
            (
                "[midia.alto_falante]\nbanco = \"sons\"\n",
                "midia.alto_falante.banco",
            ),
            (
                "[midia.microfone]\nfonte = \"simulador\"\nendereco = \"aqui\"\n",
                "midia.microfone.endereco",
            ),
            (
                "[midia.alto_falante]\nsaida = \"simulador\"\nendereco = \"aqui\"\n",
                "midia.alto_falante.endereco",
            ),
            ("[midia.alto_falante]\nvolume = 5\n", "volume"),
            ("[midia.voz]\nfolga_ms = 2001\n", "midia.voz.folga_ms"),
            ("[midia.voz]\nsilencio_ms = 100\n", "midia.voz.silencio_ms"),
            ("[midia.voz]\nsilencio_ms = 3001\n", "midia.voz.silencio_ms"),
            ("[midia.voz]\nteto_ms = 500\n", "midia.voz.teto_ms"),
            ("[midia.voz]\nteto_ms = 10001\n", "midia.voz.teto_ms"),
            (
                "[midia.voz]\nminimo_ms = 9000\nteto_ms = 8000\n",
                "midia.voz.minimo_ms",
            ),
            ("[midia.voz]\nentra = 2.0\nsai = 3.0\n", "midia.voz.entra"),
            ("[midia.voz]\nsai = 0.5\n", "midia.voz.sai"),
            ("[midia.voz]\nvolume = 1\n", "volume"),
        ] {
            let e = de(t).unwrap_err().to_string();
            assert!(e.contains(nome), "{t}: {e}");
        }
    }

    /// Parar `dji_scratch` derrubou `dji_sys` no robô.
    #[test]
    fn so_aceita_parar_o_que_a_lista_permite() {
        // Os três, por extenso: o `dji_camera` entrou com o serviço de mídia.
        assert_eq!(PODE_PARAR, ["dji_monitor", "dji_hdvt_uav", "dji_camera"]);
        de("[dji]\nparar = [\"dji_monitor\", \"dji_hdvt_uav\", \"dji_camera\"]\n").unwrap();
        for p in [
            "dji_scratch",
            "dji_sys",
            "dji_network",
            "dji_camera ",
            "dji_camera dji_scratch",
            "dji_scratch ",
            "dji_monitor dji_scratch",
            "",
        ] {
            let e = de(&format!("[dji]\nparar = [\"{p}\"]\n"))
                .unwrap_err()
                .to_string();
            assert!(e.contains("dji.parar"), "{p}: {e}");
        }
        for p in PODE_PARAR {
            de(&format!("[dji]\nparar = [\"{p}\"]\n")).unwrap();
        }
    }

    #[test]
    fn o_arquivo_do_robo_carrega() {
        let c = concat!(env!("CARGO_MANIFEST_DIR"), "/../deploy/s1/rms1d.toml");
        let p = Params::carregar(Path::new(c), true).unwrap();
        assert_eq!(p.corpo.tipo, TipoCorpo::Serial);
        assert_eq!(p.corpo.porta, "/dev/ttyS3");
        assert!(
            p.corpo.gravar_recebido.is_some(),
            "a prova em modo sombra grava o recebido"
        );
        // A tabela do robô, escrita por extenso, é a padrão: o simulador, que não a escreve, anda com a mesma.
        assert_eq!(p.fontes, Fonte::padrao());
        assert!(
            !p.autonomia.ligada,
            "o robô sobe sem andar por conta própria"
        );
    }

    const SEM_NOME: &str =
        "[[fontes]]\nnome = \"sem_nome\"\nprioridade = 0\ncanais = [\"rodas\"]\n";

    /// A ordem é a do dono: piloto, agente, cérebro; e quem não diz o nome perde para todos.
    #[test]
    fn a_tabela_padrao_e_piloto_agente_cerebro() {
        let p = Params::default();
        p.validar().unwrap();
        let mut ordem: Vec<&Fonte> = p.fontes.iter().collect();
        ordem.sort_by_key(|f| std::cmp::Reverse(f.prioridade));
        let nomes: Vec<&str> = ordem.iter().map(|f| f.nome.as_str()).collect();
        assert_eq!(nomes, ["piloto", "agente", "cerebro", FONTE_SEM_NOME]);
        let de_nome = |n: &str| p.fontes.iter().find(|f| f.nome == n).unwrap();
        assert_eq!(de_nome("piloto").canais, Canal::TODOS);
        assert_eq!(de_nome(FONTE_SEM_NOME).canais, Canal::TODOS);
        for n in ["agente", "cerebro"] {
            assert_eq!(
                de_nome(n).canais,
                [Canal::Rodas, Canal::Torre, Canal::Som],
                "{n} não arma, não muda o modo nem grava ajuste"
            );
        }
        assert!(de_nome("cerebro").so_autonomo && !de_nome("agente").so_autonomo);
        assert!(p.fontes.iter().all(|f| f.prazo_ms.is_none()));
    }

    #[test]
    fn a_tabela_do_arquivo_substitui_a_padrao() {
        let p = de(&format!(
            "{SEM_NOME}[[fontes]]\nnome = \"piloto\"\nprioridade = 9\ncanais = [\"rodas\", \"torre\"]\nprazo_ms = 250\n[autonomia]\nligada = true\n"
        ))
        .unwrap();
        assert_eq!(p.fontes.len(), 2);
        assert_eq!(p.fontes[1].prazo_ms, Some(250));
        assert_eq!(p.fontes[1].canais, [Canal::Rodas, Canal::Torre]);
        assert!(p.autonomia.ligada);
    }

    /// Cada erro da tabela é recusado dizendo o nome do que está errado.
    #[test]
    fn tabela_de_fontes_errada_e_recusada_pelo_nome() {
        let linha = |nome: &str, prioridade: u32, resto: &str| {
            format!(
                "[[fontes]]\nnome = \"{nome}\"\nprioridade = {prioridade}\ncanais = [\"rodas\"]\n{resto}"
            )
        };
        for (t, nome) in [
            // Nome repetido.
            (
                format!(
                    "{SEM_NOME}{}{}",
                    linha("piloto", 3, ""),
                    linha("piloto", 4, "")
                ),
                "o nome `piloto` está repetido",
            ),
            // Prioridade repetida.
            (
                format!(
                    "{SEM_NOME}{}{}",
                    linha("piloto", 3, ""),
                    linha("agente", 3, "")
                ),
                "`piloto` e `agente` têm a mesma prioridade (3)",
            ),
            // Canal desconhecido.
            (
                format!(
                    "{SEM_NOME}[[fontes]]\nnome = \"piloto\"\nprioridade = 3\ncanais = [\"asas\"]\n"
                ),
                "asas",
            ),
            // Falta a linha de quem não diz o nome.
            (linha("piloto", 3, ""), "falta a linha `sem_nome`"),
            // Prazo fora da faixa, nas duas pontas.
            (
                format!("{SEM_NOME}{}", linha("piloto", 3, "prazo_ms = 0\n")),
                "prazo_ms de `piloto` fora de 1 a 10000: 0",
            ),
            (
                format!("{SEM_NOME}{}", linha("piloto", 3, "prazo_ms = 10001\n")),
                "prazo_ms de `piloto` fora de 1 a 10000: 10001",
            ),
            // Chave escrita errada numa linha.
            (
                format!("{SEM_NOME}{}", linha("piloto", 3, "prazzo_ms = 250\n")),
                "prazzo_ms",
            ),
            // Linha sem nome.
            (
                format!("{SEM_NOME}{}", linha("", 3, "")),
                "a linha 2 está sem nome",
            ),
            ("[autonomia]\nligado = true\n".to_owned(), "ligado"),
        ] {
            let e = de(&t).unwrap_err().to_string();
            assert!(e.contains(nome), "{t}: {e}");
        }
        // Mais linhas do que o teto.
        let muitas: String = (1..=FONTES_MAX as u32)
            .map(|i| linha(&format!("f{i}"), i, ""))
            .collect();
        let e = de(&format!("{SEM_NOME}{muitas}")).unwrap_err().to_string();
        assert!(e.contains("o teto é 16"), "{e}");
    }
}
