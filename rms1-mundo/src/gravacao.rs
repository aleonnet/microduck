//! Gravar uma sessão e reproduzi-la: as grades como chegaram, as amostras da torre e da pose, e a câmera, cada registro com a
//! hora. A reprodução passa o arquivo pelo mesmo caminho do robô e devolve o mundo de cada instante: é como a calibração e os
//! testes rodam em dado do robô sem o robô.
//!
//! O arquivo: a linha `rms1-mundo 1`, e depois os registros, cada um com o tipo (um byte), a hora (f64), o tamanho (u32) e a
//! carga, tudo com o byte menor primeiro.
//!   `c`  a câmera e o corpo, em JSON; é o primeiro registro
//!   `a`  uma amostra: o giro e a inclinação da torre, e x, y e o rumo da pose (cinco f64)
//!   `g`  uma grade: o tamanho do nome do leitor (um byte), o nome, e a entrega como chegou; a hora é a da captura
//!   `f`  uma falha dita pela estação: o motivo, em UTF-8
//!
//! Uma grade do `scdepthv3` tem 115 kB, e a 15 por segundo a sessão cresce 104 MB por minuto: a gravação para sozinha ao
//! chegar em `gravar_limite_mb`, e o serviço segue.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::folga::Corpo;
use crate::geometria::Olho;
use crate::historia::Amostra;
use crate::ocupacao::Estado;
use crate::{Agora, Ajustes, Mundo, Tratada};

const MARCA: &[u8] = b"rms1-mundo 1\n";

#[derive(Serialize, Deserialize)]
struct Cabecalho {
    olho: Option<Olho>,
    largura_m: f64,
    frente_m: f64,
}

pub struct Gravador {
    saida: Option<BufWriter<File>>,
    escritos: u64,
    limite: u64,
}

impl Gravador {
    /// Cria o arquivo, por cima do que houver, e escreve a câmera e o corpo.
    pub fn criar(
        caminho: &Path,
        olho: Option<&Olho>,
        corpo: &Corpo,
        limite_mb: u64,
    ) -> std::io::Result<Self> {
        let mut saida = BufWriter::new(File::create(caminho)?);
        saida.write_all(MARCA)?;
        let mut eu = Self {
            saida: Some(saida),
            escritos: MARCA.len() as u64,
            limite: limite_mb * 1024 * 1024,
        };
        let cabecalho = Cabecalho {
            olho: olho.copied(),
            largura_m: corpo.largura_m,
            frente_m: corpo.frente_m,
        };
        eu.registro(b'c', 0.0, &[&serde_json::to_vec(&cabecalho)?]);
        match &mut eu.saida {
            Some(saida) => saida.flush()?,
            None => return Err(std::io::Error::other("a gravação não coube no limite")),
        }
        Ok(eu)
    }

    pub fn no_ar(&self) -> bool {
        self.saida.is_some()
    }

    /// Um registro. O que não cabe no limite, ou não se escreve, encerra a gravação: o serviço não para por ela.
    fn registro(&mut self, tipo: u8, t: f64, partes: &[&[u8]]) {
        let Some(saida) = &mut self.saida else { return };
        let tamanho: usize = partes.iter().map(|p| p.len()).sum();
        let total = 1 + 8 + 4 + tamanho as u64;
        let escrito = (|| {
            if self.escritos + total > self.limite || tamanho > u32::MAX as usize {
                return Err(std::io::Error::other("o limite da gravação"));
            }
            saida.write_all(&[tipo])?;
            saida.write_all(&t.to_le_bytes())?;
            saida.write_all(&(tamanho as u32).to_le_bytes())?;
            for parte in partes {
                saida.write_all(parte)?;
            }
            Ok(())
        })();
        match escrito {
            Ok(()) => self.escritos += total,
            Err(_) => {
                let _ = saida.flush();
                self.saida = None;
            }
        }
    }

    pub fn amostra(&mut self, a: &Amostra) {
        let mut carga = [0u8; 40];
        for (k, v) in a.torre.iter().chain(a.pose.iter()).enumerate() {
            carga[k * 8..k * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
        self.registro(b'a', a.t, &[&carga]);
    }

    pub fn grade(&mut self, t_captura: f64, leitor: &str, carga: &[u8]) {
        let nome = &leitor.as_bytes()[..leitor.len().min(255)];
        self.registro(b'g', t_captura, &[&[nome.len() as u8], nome, carga]);
        // Uma grade por vez no disco: a sessão interrompida perde, no máximo, a última.
        if let Some(saida) = &mut self.saida
            && saida.flush().is_err()
        {
            self.saida = None;
        }
    }

    pub fn falha(&mut self, t: f64, motivo: &str) {
        self.registro(b'f', t, &[motivo.as_bytes()]);
    }
}

/// Um registro lido de uma gravação.
#[derive(Debug, Clone, PartialEq)]
pub enum Registro {
    Amostra(Amostra),
    Grade {
        t_captura: f64,
        leitor: String,
        carga: Vec<u8>,
    },
    Falha {
        t: f64,
        motivo: String,
    },
}

/// Uma gravação lida inteira: a câmera, o corpo e os registros, na ordem.
#[derive(Debug, Clone, PartialEq)]
pub struct Sessao {
    pub olho: Option<Olho>,
    pub corpo: Corpo,
    pub registros: Vec<Registro>,
}

/// Lê a gravação. O registro cortado no fim (a sessão interrompida) encerra a leitura sem erro; o que não é uma gravação é
/// recusado pelo motivo.
pub fn ler(caminho: &Path) -> Result<Sessao, String> {
    let mut bytes = Vec::new();
    File::open(caminho)
        .and_then(|mut f| f.read_to_end(&mut bytes))
        .map_err(|e| format!("não consegui ler {}: {e}", caminho.display()))?;
    let Some(mut resto) = bytes.strip_prefix(MARCA) else {
        return Err(format!(
            "{} não é uma gravação do rms1-mundo",
            caminho.display()
        ));
    };
    let mut cabecalho: Option<Cabecalho> = None;
    let mut registros = Vec::new();
    while resto.len() >= 13 {
        let tipo = resto[0];
        let t = f64::from_le_bytes(resto[1..9].try_into().expect("oito bytes"));
        let tamanho = u32::from_le_bytes(resto[9..13].try_into().expect("quatro bytes")) as usize;
        if resto.len() < 13 + tamanho {
            break;
        }
        let carga = &resto[13..13 + tamanho];
        resto = &resto[13 + tamanho..];
        match tipo {
            b'c' => {
                cabecalho = Some(
                    serde_json::from_slice(carga)
                        .map_err(|e| format!("a câmera da gravação não se lê: {e}"))?,
                );
            }
            b'a' if tamanho == 40 => {
                let v = |k: usize| {
                    f64::from_le_bytes(carga[k * 8..k * 8 + 8].try_into().expect("oito bytes"))
                };
                registros.push(Registro::Amostra(Amostra {
                    t,
                    torre: [v(0), v(1)],
                    pose: [v(2), v(3), v(4)],
                }));
            }
            b'g' if tamanho >= 1 && tamanho > usize::from(carga[0]) => {
                let n = usize::from(carga[0]);
                registros.push(Registro::Grade {
                    t_captura: t,
                    leitor: String::from_utf8_lossy(&carga[1..1 + n]).into_owned(),
                    carga: carga[1 + n..].to_vec(),
                });
            }
            b'f' => registros.push(Registro::Falha {
                t,
                motivo: String::from_utf8_lossy(carga).into_owned(),
            }),
            outro => {
                return Err(format!(
                    "a gravação tem um registro de tipo {outro} com {tamanho} bytes, que não conheço"
                ));
            }
        }
    }
    let cabecalho = cabecalho.ok_or_else(|| "a gravação não traz a câmera".to_owned())?;
    Ok(Sessao {
        olho: cabecalho.olho,
        corpo: Corpo {
            largura_m: cabecalho.largura_m,
            frente_m: cabecalho.frente_m,
        },
        registros,
    })
}

/// Um passo da reprodução: o mundo depois de uma grade ou de uma falha.
#[derive(Debug, Clone, PartialEq)]
pub struct Passo {
    pub t: f64,
    pub agora: Agora,
    /// O que a grade deste passo deu; nada, se o passo é uma falha dita pela estação.
    pub tratada: Option<Tratada>,
    /// O meio de cada célula que este quadro marcou como ocupada, na odometria, em metros.
    pub ocupadas: Vec<[f64; 2]>,
}

/// Passa a sessão pelo mesmo caminho do robô, com estes ajustes, e devolve o mundo depois de cada grade e de cada falha, com
/// a hora. `olho` troca a câmera da gravação: é como a calibração experimenta uma lente sem voltar ao robô.
pub fn reproduzir(
    sessao: &Sessao,
    olho: Option<Olho>,
    ajustes: &Ajustes,
) -> Result<Vec<(f64, Agora)>, String> {
    Ok(reproduzir_em_passos(sessao, olho, ajustes)?
        .into_iter()
        .map(|p| (p.t, p.agora))
        .collect())
}

/// A mesma reprodução, com o que cada grade deu: a escala e o exame dela, e as células que ela marcou como ocupadas. É o que
/// a calibração mede.
pub fn reproduzir_em_passos(
    sessao: &Sessao,
    olho: Option<Olho>,
    ajustes: &Ajustes,
) -> Result<Vec<Passo>, String> {
    // A reprodução não grava por cima da sessão que lê.
    let ajustes = Ajustes {
        gravar: String::new(),
        ..ajustes.clone()
    };
    let lado = ajustes.celula_m;
    let mut mundo = Mundo::novo(olho.or(sessao.olho), sessao.corpo, ajustes)
        .map_err(|e| format!("o mundo não abriu: {e}"))?;
    let mut saida = Vec::new();
    for registro in &sessao.registros {
        match registro {
            Registro::Amostra(a) => mundo.amostra(*a),
            Registro::Grade {
                t_captura,
                leitor,
                carga,
            } => {
                let tratada = mundo.grade(carga, leitor, *t_captura)?;
                // As ocupadas deste quadro: as que ele acabou de marcar, com a idade em zero.
                let ocupadas = mundo
                    .celulas(*t_captura)
                    .into_iter()
                    .filter(|(_, estado, idade)| *estado == Estado::Ocupada && *idade == 0.0)
                    .map(|(c, _, _)| {
                        [
                            (f64::from(c[0]) + 0.5) * lado,
                            (f64::from(c[1]) + 0.5) * lado,
                        ]
                    })
                    .collect();
                saida.push(Passo {
                    t: *t_captura,
                    agora: mundo.agora(*t_captura),
                    tratada: Some(tratada),
                    ocupadas,
                });
            }
            Registro::Falha { t, motivo } => {
                mundo.falha(*t, motivo);
                saida.push(Passo {
                    t: *t,
                    agora: mundo.agora(*t),
                    tratada: None,
                    ocupadas: Vec::new(),
                });
            }
        }
    }
    Ok(saida)
}
