//! A tabela das habilidades: `habilidades.toml`. Chave desconhecida é recusada pelo nome, como em `rms1d.toml`.

use std::collections::BTreeMap;

use serde::Deserialize;

/// A tabela que vem com o agente.
const PADRAO: &str = include_str!("../habilidades.toml");

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tabela {
    pub geral: Geral,
    #[serde(default, rename = "habilidade")]
    pub habilidades: Vec<Habilidade>,
    /// Palavras do pedido que querem dizer um rótulo do detector.
    #[serde(default)]
    pub alvos: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Geral {
    /// De quanto em quanto o pedido contínuo é renovado.
    pub renovar_ms: u64,
    /// Sem o estado do corpo por este tempo, o `rms1d` saiu.
    pub corpo_vale_ms: u64,
    /// Com o canal da habilidade (as rodas, ou a torre) na mão de outra fonte por este tempo, o agente a dá por
    /// interrompida. Ausente no arquivo, vale [`PERDEU_APOS_PADRAO_MS`].
    #[serde(default = "perdeu_apos_padrao")]
    pub perdeu_apos_ms: u64,
}

/// O `perdeu_apos_ms` de uma tabela que não o diz: o mesmo de `habilidades.toml`, e um teste reprova se os dois se afastam.
pub const PERDEU_APOS_PADRAO_MS: u64 = 300;

fn perdeu_apos_padrao() -> u64 {
    PERDEU_APOS_PADRAO_MS
}

/// Uma habilidade: a ferramenta da estação que ela cumpre, e os números dela.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "faz", rename_all = "snake_case", deny_unknown_fields)]
pub enum Habilidade {
    Parar {
        ferramenta: String,
    },
    Mover {
        ferramenta: String,
        /// O parâmetro que diz a direção, e o que diz a distância, em metros.
        direcao: String,
        metros: String,
        /// Metros por segundo.
        velocidade: f64,
        /// O teto é o tempo da distância vezes isto, mais um segundo.
        folga: f64,
        /// O que cada direção é no corpo: x à frente, y à esquerda.
        direcoes: BTreeMap<String, [f64; 2]>,
    },
    Olhar {
        ferramenta: String,
        /// O parâmetro que diz o alvo.
        alvo: String,
        /// Graus por segundo com o alvo na borda do quadro.
        ganho: f64,
        /// A que distância do meio, em fração do quadro, o alvo está no meio.
        tolerancia: f64,
        /// Quantas detecções seguidas no meio terminam.
        seguidas: u32,
        teto_s: f64,
    },
    Procurar {
        ferramenta: String,
        alvo: String,
        /// Graus por segundo.
        varredura: f64,
        /// Quanto tempo a torre vai para cada lado de onde começou.
        lado_s: f64,
        teto_s: f64,
    },
}

impl Habilidade {
    pub fn ferramenta(&self) -> &str {
        match self {
            Self::Parar { ferramenta }
            | Self::Mover { ferramenta, .. }
            | Self::Olhar { ferramenta, .. }
            | Self::Procurar { ferramenta, .. } => ferramenta,
        }
    }
}

impl Tabela {
    /// A tabela que vem com o agente.
    pub fn padrao() -> Result<Self, String> {
        Self::ler(PADRAO)
    }

    /// Lê e confere a tabela. O erro diz o nome do que está errado.
    pub fn ler(texto: &str) -> Result<Self, String> {
        let t: Self = toml::from_str(texto).map_err(|e| e.to_string())?;
        t.validar()?;
        Ok(t)
    }

    fn validar(&self) -> Result<(), String> {
        let positivo = |nome: &str, v: f64| {
            (v.is_finite() && v > 0.0)
                .then_some(())
                .ok_or_else(|| format!("{nome} tem de ser maior que zero, e é {v}"))
        };
        if self.geral.renovar_ms == 0 {
            return Err("geral.renovar_ms não pode ser 0".to_owned());
        }
        if self.geral.corpo_vale_ms == 0 {
            return Err("geral.corpo_vale_ms não pode ser 0".to_owned());
        }
        if self.geral.perdeu_apos_ms == 0 {
            return Err("geral.perdeu_apos_ms não pode ser 0".to_owned());
        }
        let mut vistas = Vec::new();
        for h in &self.habilidades {
            let f = h.ferramenta();
            if f.is_empty() {
                return Err("uma habilidade está sem ferramenta".to_owned());
            }
            if vistas.contains(&f) {
                return Err(format!("a ferramenta {f} tem duas habilidades"));
            }
            vistas.push(f);
            match h {
                Habilidade::Parar { .. } => {}
                Habilidade::Mover {
                    velocidade,
                    folga,
                    direcoes,
                    ..
                } => {
                    positivo(&format!("{f}: velocidade"), *velocidade)?;
                    if !(folga.is_finite() && *folga >= 1.0) {
                        return Err(format!("{f}: folga tem de ser 1 ou mais, e é {folga}"));
                    }
                    if direcoes.is_empty() {
                        return Err(format!("{f}: direcoes está vazio"));
                    }
                    for (nome, [x, y]) in direcoes {
                        if !(x.is_finite() && y.is_finite()) || x.hypot(*y) == 0.0 {
                            return Err(format!(
                                "{f}: a direção {nome} não aponta para lugar nenhum"
                            ));
                        }
                    }
                }
                Habilidade::Olhar {
                    ganho,
                    tolerancia,
                    seguidas,
                    teto_s,
                    ..
                } => {
                    positivo(&format!("{f}: ganho"), *ganho)?;
                    positivo(&format!("{f}: teto_s"), *teto_s)?;
                    if !(tolerancia.is_finite() && *tolerancia > 0.0 && *tolerancia < 0.5) {
                        return Err(format!(
                            "{f}: tolerancia tem de ficar entre 0 e 0,5 do quadro, e é {tolerancia}"
                        ));
                    }
                    if *seguidas == 0 {
                        return Err(format!("{f}: seguidas não pode ser 0"));
                    }
                }
                Habilidade::Procurar {
                    varredura,
                    lado_s,
                    teto_s,
                    ..
                } => {
                    positivo(&format!("{f}: varredura"), *varredura)?;
                    positivo(&format!("{f}: lado_s"), *lado_s)?;
                    positivo(&format!("{f}: teto_s"), *teto_s)?;
                }
            }
        }
        if !self
            .habilidades
            .iter()
            .any(|h| matches!(h, Habilidade::Parar { .. }))
        {
            return Err("a tabela não tem a habilidade de parar (faz = \"parar\")".to_owned());
        }
        Ok(())
    }

    /// Confere a tabela com o corpo que o `rms1d` comanda: o prazo sem pedido novo (`[seguranca] prazo_ms`) e a velocidade
    /// máxima da tabela física.
    pub fn conferir_com_o_corpo(&self, prazo_ms: u64, velocidade_max: f64) -> Result<(), String> {
        if self.geral.renovar_ms * 2 > prazo_ms {
            return Err(format!(
                "geral.renovar_ms é {}, e o rms1d zera o corpo sem pedido novo por {prazo_ms} ms: têm de caber duas renovações no prazo",
                self.geral.renovar_ms
            ));
        }
        for h in &self.habilidades {
            if let Habilidade::Mover {
                ferramenta,
                velocidade,
                ..
            } = h
                && *velocidade > velocidade_max
            {
                return Err(format!(
                    "{ferramenta}: velocidade é {velocidade} m/s, e o corpo vai até {velocidade_max}"
                ));
            }
        }
        Ok(())
    }
}
