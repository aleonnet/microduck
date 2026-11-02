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

/// O único que pode ser parado no robô, os dois que seguram a serial. Lista de permitidos, e não de proibidos: parar
/// `dji_scratch` derrubou `dji_sys` (`docs/s1/robot/rms1d-pontos-de-controle.md`), e o próximo nome ninguém conhece.
const PODE_PARAR: [&str; 2] = ["dji_monitor", "dji_hdvt_uav"];

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
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
    pub dji: Dji,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Seguranca {
    /// O mesmo do pato (`duck-control/src/safety.rs`): sem comando novo, as rodas zeram.
    pub prazo_ms: u64,
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Dji {
    /// Processos que seguram a serial (`scripts/s1/bench.sh:147`).
    pub parar: Vec<String>,
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
            hz: 50,
            hz_minimo: 40.0,
            idade_maxima_ms: 500,
        }
    }
}

impl Default for Seguranca {
    fn default() -> Self {
        Self {
            prazo_ms: 500,
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
        assert_eq!(p.laco.hz, 50, "o resto segue o padrão");
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
        ] {
            assert!(de(t).is_err(), "{t}");
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

    /// Parar `dji_scratch` derrubou `dji_sys` no robô.
    #[test]
    fn so_aceita_parar_o_que_segura_a_serial() {
        for p in [
            "dji_scratch",
            "dji_sys",
            "dji_network",
            "dji_camera",
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
    }
}
