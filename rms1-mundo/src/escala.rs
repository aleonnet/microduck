//! A escala pelo chão: o que leva a profundidade do `scdepthv3`, que vem a menos de uma escala, aos metros.
//!
//! A câmera (a lente, a altura dela e a inclinação da torre no instante do quadro) diz a profundidade que o chão tem em cada
//! ponto abaixo do horizonte. A grade diz o inverso da profundidade, `u`, a menos de uma escala: a profundidade em metros é
//! `escala / u`. Num ponto que é chão, `escala = z_chao * u`. O que está mais perto do que o chão estaria ali (um objeto) tem
//! `u` maior, e puxa essa razão para cima: por isso a escala é procurada no quartil de baixo das razões da faixa de baixo do
//! quadro, e é a mediana das que ficam perto dele.
//!
//! O chão vem da geometria, e não de um ajuste livre de um plano à grade: medido na grade guardada em 2026-11-20, o horizonte
//! de um ajuste livre vai da linha 11 à 108 de 180 conforme a faixa que se toma (no laboratório,
//! `out/2026-11-20-distancia-na-estacao/reta_do_chao.py`).
//!
//! Duas conferências antes de a escala de um quadro valer. A primeira é a fração da faixa que ficou na conta. A segunda é a
//! escala sair igual por dois caminhos: pela razão, acima, e pela inclinação da reta do inverso da profundidade contra a linha
//! do quadro, que no chão vale `escala * cos(inclinação) / altura` e não depende de onde o horizonte está. Com a faixa cheia
//! de chão e a inclinação da torre certa, as duas dão o mesmo número. Com uma parede enchendo a faixa (o inverso não muda com
//! a linha) ou com a grade casada com a inclinação errada (o horizonte está noutro lugar), elas se separam, e a distância
//! entre elas é o próprio erro que a escala teria: o quadro só é dado por medido com esse erro dentro da tolerância.

use std::collections::VecDeque;

use crate::Ajustes;
use crate::geometria::{Raios, Vista};
use crate::grade::{Conversor, Grade};

/// A escala achada num quadro, com a qualidade dela.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DoQuadro {
    pub valor: f64,
    /// A fração da faixa de baixo que entrou na conta como chão.
    pub chao: f64,
    /// A dispersão das razões que entraram: a mediana do desvio, em fração da escala.
    pub dispersao: f64,
    /// Quanto a escala pela razão difere da escala pela inclinação da reta, em fração: o erro que ela teria se o que a faixa
    /// mostra não fosse o chão que a conta espera. No chão é perto de zero.
    pub desvio: f64,
}

fn mediana(valores: &mut [f32]) -> f32 {
    let meio = valores.len() / 2;
    *valores.select_nth_unstable_by(meio, f32::total_cmp).1
}

/// A escala deste quadro. Nenhuma, se a faixa de baixo não tem chão bastante ou o que ela mostra não é chão.
pub fn do_quadro(
    g: &Grade<'_>,
    conversor: &Conversor,
    vista: &Vista,
    raios: &Raios,
    ajustes: &Ajustes,
) -> Option<DoQuadro> {
    let de = ((ajustes.semente_de * g.altura as f64).ceil() as usize).min(g.altura);
    let total = (g.altura - de) * g.largura;
    let minimo = ((ajustes.chao_minimo * total as f64).ceil() as usize).max(8);
    let altura = vista.altura;
    // Cada ponto da faixa que pode ser chão: a linha dele, a razão entre a profundidade do chão ali e a da grade, e o inverso
    // da profundidade que a grade diz.
    let mut pontos: Vec<(f32, f32, f32)> = Vec::with_capacity(total);
    for j in de..g.altura {
        let denominador = vista.denominador(raios.yn[j]);
        if denominador <= 0.0 {
            continue;
        }
        let z_chao = altura / denominador;
        if z_chao > ajustes.alcance_m {
            continue;
        }
        for i in 0..g.largura {
            let u = conversor.inverso(g, j * g.largura + i);
            if u > 0.0 {
                pontos.push((raios.yn[j] as f32, (z_chao * f64::from(u)) as f32, u));
            }
        }
    }
    if pontos.len() < minimo {
        return None;
    }
    let mut razoes: Vec<f32> = pontos.iter().map(|p| p.1).collect();
    let quarto = razoes.len() / 4;
    let de_baixo = *razoes.select_nth_unstable_by(quarto, f32::total_cmp).1;
    let folga = ajustes.tolerancia as f32 * de_baixo;
    pontos.retain(|p| (p.1 - de_baixo).abs() <= folga);
    if pontos.len() < minimo {
        return None;
    }
    let mut razoes: Vec<f32> = pontos.iter().map(|p| p.1).collect();
    let valor = f64::from(mediana(&mut razoes));
    // A reta do inverso da profundidade contra a linha, por mínimos quadrados, e a escala que a inclinação dela dá.
    let n = pontos.len() as f64;
    let (mut sy, mut su, mut syy, mut syu) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for &(yn, _, u) in &pontos {
        let (y, u) = (f64::from(yn), f64::from(u));
        sy += y;
        su += u;
        syy += y * y;
        syu += y * u;
    }
    let espalhamento = n * syy - sy * sy;
    if espalhamento <= 1e-12 {
        return None;
    }
    let pela_reta = (n * syu - sy * su) / espalhamento * vista.escala_por_inclinacao_da_reta();
    // A reta deitada ou descendo (uma parede enchendo a faixa) não é chão.
    if pela_reta.is_nan() || pela_reta <= 0.0 {
        return None;
    }
    let desvio = (valor / pela_reta - 1.0).abs();
    if desvio > ajustes.tolerancia {
        return None;
    }
    let mut desvios: Vec<f32> = razoes.iter().map(|r| (r - valor as f32).abs()).collect();
    Some(DoQuadro {
        valor,
        chao: pontos.len() as f64 / total as f64,
        dispersao: f64::from(mediana(&mut desvios)) / valor,
        desvio,
    })
}

/// As escalas boas dos últimos quadros, para o quadro em que o chão sai da faixa de baixo (a torre inclinada para cima, um
/// objeto largo muito perto): vale a mediana das de antes, por um prazo, e quem a usa diz que ela é lembrada.
#[derive(Debug, Default)]
pub struct Memoria(VecDeque<(f64, f64)>);

/// Quantas escalas ficam guardadas: a 15 quadros por segundo, pouco mais de quatro segundos.
const GUARDADAS: usize = 64;

impl Memoria {
    pub fn guardar(&mut self, t: f64, escala: f64) {
        if self.0.len() == GUARDADAS {
            self.0.pop_front();
        }
        self.0.push_back((t, escala));
    }

    /// A mediana das escalas boas dos últimos `vale_ms`. Nenhuma, se não houve escala boa nesse prazo.
    pub fn lembrada(&self, t: f64, vale_ms: u64) -> Option<f64> {
        let prazo = vale_ms as f64 / 1000.0;
        let mut recentes: Vec<f64> = self
            .0
            .iter()
            .filter(|(quando, _)| (0.0..=prazo).contains(&(t - quando)))
            .map(|(_, e)| *e)
            .collect();
        if recentes.is_empty() {
            return None;
        }
        let meio = recentes.len() / 2;
        Some(*recentes.select_nth_unstable_by(meio, f64::total_cmp).1)
    }
}
