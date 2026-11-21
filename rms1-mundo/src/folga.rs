//! O que o mapa diz a quem vai andar: a folga por direção em volta do corpo, e o obstáculo na forma do cérebro do pato.
//!
//! As duas contas saem do mapa, que mora na odometria, reexpresso no corpo de agora: o que foi visto chega mais perto quando
//! o robô anda na direção dele, sem quadro novo.
//!
//! - **A folga:** o corpo é cercado de setores iguais, o primeiro para a frente e os seguintes para a esquerda. Cada um diz a
//!   distância, do centro do corpo, até a célula ocupada mais perto; sem ocupada, até onde o chão foi visto livre sem
//!   interrupção; ou que nada foi visto ali. A idade é a da observação que deu o número.
//! - **O obstáculo** (`autod/src/obstacle.rs`, `Obstacle`): a folga da frente do chassi até o ocupado mais perto no corredor
//!   que o corpo varre andando para a frente, o lado mais livre, e há quanto tempo o corredor foi visto.

use crate::Ajustes;
use crate::geometria::odom_para_corpo;
use crate::ocupacao::{Estado, Mapa};

/// A folga numa direção.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Setor {
    /// O rumo do meio do setor, em graus no corpo: zero à frente, positivo à esquerda, de -180 a 180.
    pub rumo_grau: f64,
    /// `Ocupada`: há um ocupado a `livre_m`. `Livre`: o chão foi visto livre até `livre_m`. `NaoVista`: nada foi visto.
    pub estado: Estado,
    pub livre_m: f64,
    pub idade_s: f64,
}

/// O caminho à frente, no corpo de agora.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Obstaculo {
    /// Da frente do chassi ao ocupado mais perto no corredor, em metros; infinito com o corredor livre.
    pub ahead_m: f64,
    /// Para que lado virar: +1 à esquerda, -1 à direita. O lado em que o ocupado mais perto está mais longe.
    pub away: f64,
    /// Há quantos segundos o corredor foi visto pela última vez.
    pub age_s: f64,
}

/// O corpo, para as contas do caminho: da tabela física (`dimensoes`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corpo {
    pub largura_m: f64,
    pub frente_m: f64,
}

/// A folga por direção e o obstáculo, do mapa visto da `pose` no instante `t`. O obstáculo não existe enquanto o corredor
/// nunca foi visto.
pub fn do_mapa(
    mapa: &Mapa,
    pose: [f64; 3],
    t: f64,
    corpo: &Corpo,
    ajustes: &Ajustes,
) -> (Vec<Setor>, Option<Obstaculo>) {
    let n = ajustes.setores as usize;
    let passo = std::f64::consts::TAU / n as f64;
    let lado = mapa.lado();
    let faixas = (ajustes.alcance_m / lado).ceil() as usize + 2;
    // Por setor: o ocupado mais perto com a idade dele; e, por faixa de distância, a idade do chão livre mais novo.
    let mut ocupado: Vec<Option<(f64, f64)>> = vec![None; n];
    let mut livre: Vec<Vec<Option<f64>>> = vec![vec![None; faixas]; n];
    // O corredor: meia largura do corpo e mais uma célula, que é o que o mapa distingue.
    let meio_corredor = corpo.largura_m / 2.0 + lado;
    let mut ahead_m = f64::INFINITY;
    let (mut esquerda, mut direita) = (f64::INFINITY, f64::INFINITY);
    let mut corredor_visto: Option<f64> = None;
    for (c, estado, idade) in mapa.vivas(t, ajustes.lembra_s) {
        let [x, y] = odom_para_corpo(pose, mapa.meio_de(c));
        let d = x.hypot(y);
        if d > ajustes.alcance_m + lado {
            continue;
        }
        let k = ((y.atan2(x) / passo).round() as i64).rem_euclid(n as i64) as usize;
        match estado {
            Estado::Ocupada => {
                if ocupado[k].is_none_or(|(perto, _)| d < perto) {
                    ocupado[k] = Some((d, idade));
                }
            }
            Estado::Livre => {
                let faixa = ((d / lado) as usize).min(faixas - 1);
                if livre[k][faixa].is_none_or(|i| idade < i) {
                    livre[k][faixa] = Some(idade);
                }
            }
            Estado::NaoVista => {}
        }
        if x > 0.0 {
            if y.abs() < meio_corredor {
                corredor_visto = Some(corredor_visto.map_or(idade, |i: f64| i.min(idade)));
                if estado == Estado::Ocupada {
                    ahead_m = ahead_m.min((x - corpo.frente_m).max(0.0));
                }
            }
            if estado == Estado::Ocupada {
                if y >= 0.0 {
                    esquerda = esquerda.min(d);
                } else {
                    direita = direita.min(d);
                }
            }
        }
    }
    let setores = (0..n)
        .map(|k| {
            let rumo = k as f64 * passo;
            let rumo_grau = if rumo > std::f64::consts::PI {
                rumo - std::f64::consts::TAU
            } else {
                rumo
            }
            .to_degrees();
            if let Some((d, idade)) = ocupado[k] {
                return Setor {
                    rumo_grau,
                    estado: Estado::Ocupada,
                    livre_m: d,
                    idade_s: idade,
                };
            }
            // Do chão visto mais perto para longe, enquanto não há interrupção de mais de duas faixas.
            let (mut ate, mut idade, mut vazias) = (None, f64::INFINITY, 0usize);
            for (faixa, vista) in livre[k].iter().enumerate() {
                match vista {
                    Some(i) => {
                        ate = Some(faixa);
                        idade = idade.min(*i);
                        vazias = 0;
                    }
                    None if ate.is_some() => {
                        vazias += 1;
                        if vazias > 2 {
                            break;
                        }
                    }
                    None => {}
                }
            }
            match ate {
                Some(faixa) => Setor {
                    rumo_grau,
                    estado: Estado::Livre,
                    livre_m: ((faixa + 1) as f64 * lado).min(ajustes.alcance_m),
                    idade_s: idade,
                },
                None => Setor {
                    rumo_grau,
                    estado: Estado::NaoVista,
                    livre_m: 0.0,
                    idade_s: f64::INFINITY,
                },
            }
        })
        .collect();
    let obstaculo = corredor_visto.map(|age_s| Obstaculo {
        ahead_m,
        away: if esquerda >= direita { 1.0 } else { -1.0 },
        age_s,
    });
    (setores, obstaculo)
}
