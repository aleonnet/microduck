//! A câmera e as mudanças de referencial: do ponto do quadro ao chão em volta do robô, e a volta.
//!
//! Os referenciais, todos com x para a frente, y para a esquerda e z para cima, em metros:
//! - **a lente:** um ponto do quadro, em coordenadas normalizadas (`xn` para a direita, `yn` para baixo, com a profundidade
//!   valendo 1), é o raio `[1, -xn, -yn]`. A profundidade `z` é medida ao longo do eixo da lente, como a dos modelos;
//! - **a torre nivelada:** a lente girada pela inclinação, positiva para cima, em volta do eixo de inclinação da torre;
//! - **o corpo:** a torre girada pelo giro dela, positivo à esquerda. O eixo da torre é tomado no centro do corpo (a malha do
//!   app o põe a 5 mm dele);
//! - **a odometria:** o corpo posto na pose `[x, y, rumo]`.
//!
//! Nenhum número do robô mora aqui: a lente e a posição dela vêm da tabela física, por quem chama.

use rms1d_params::{Camera, CameraNoCorpo};
use serde::{Deserialize, Serialize};

use crate::historia::Amostra;

/// A câmera inteira: a lente e onde ela fica no corpo.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Olho {
    pub lente: Camera,
    pub no_corpo: CameraNoCorpo,
}

impl Olho {
    /// O ponto `(u, v)` do quadro da lente, normalizado: `[xn, yn]`.
    pub fn normalizado(&self, u: f64, v: f64) -> [f64; 2] {
        let l = &self.lente;
        [(u - l.cx) / l.fx, (v - l.cy) / l.fy]
    }

    /// A volta: o ponto do quadro da lente em que `[xn, yn]` cai.
    pub fn no_quadro(&self, xn: f64, yn: f64) -> [f64; 2] {
        let l = &self.lente;
        [l.cx + l.fx * xn, l.cy + l.fy * yn]
    }

    /// O campo de visão, em graus: o horizontal e o vertical.
    pub fn campo_grau(&self) -> [f64; 2] {
        let l = &self.lente;
        let abertura = |de: f64, ate: f64, foco: f64| (de / foco).atan() + (ate / foco).atan();
        [
            abertura(l.cx, l.largura - l.cx, l.fx).to_degrees(),
            abertura(l.cy, l.altura - l.cy, l.fy).to_degrees(),
        ]
    }
}

/// O centro de cada coluna e de cada linha de uma grade, normalizado. A grade cobre o quadro inteiro da lente, seja qual for o
/// tamanho dela.
#[derive(Debug, Clone, PartialEq)]
pub struct Raios {
    pub xn: Vec<f64>,
    pub yn: Vec<f64>,
}

impl Raios {
    pub fn da_grade(olho: &Olho, largura: usize, altura: usize) -> Self {
        let l = &olho.lente;
        Self {
            xn: (0..largura)
                .map(|i| ((i as f64 + 0.5) * l.largura / largura as f64 - l.cx) / l.fx)
                .collect(),
            yn: (0..altura)
                .map(|j| ((j as f64 + 0.5) * l.altura / altura as f64 - l.cy) / l.fy)
                .collect(),
        }
    }

    pub fn serve(&self, largura: usize, altura: usize) -> bool {
        self.xn.len() == largura && self.yn.len() == altura
    }
}

/// Um ponto do corpo, levado à odometria.
pub fn corpo_para_odom(pose: [f64; 3], p: [f64; 2]) -> [f64; 2] {
    let (s, c) = pose[2].sin_cos();
    [pose[0] + c * p[0] - s * p[1], pose[1] + s * p[0] + c * p[1]]
}

/// Um ponto da odometria, visto do corpo.
pub fn odom_para_corpo(pose: [f64; 3], p: [f64; 2]) -> [f64; 2] {
    let (s, c) = pose[2].sin_cos();
    let (dx, dy) = (p[0] - pose[0], p[1] - pose[1]);
    [c * dx + s * dy, -s * dx + c * dy]
}

/// A câmera num instante: a lente com a torre e o corpo onde a amostra os diz.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vista {
    sen_i: f64,
    cos_i: f64,
    sen_g: f64,
    cos_g: f64,
    /// A altura da lente acima do chão, com a inclinação deste instante.
    pub altura: f64,
    /// Quanto a lente está à frente do eixo da torre, na torre nivelada.
    a_frente: f64,
    pub pose: [f64; 3],
}

impl Vista {
    pub fn nova(olho: &Olho, a: &Amostra) -> Self {
        let inclinacao = a.torre[1] + olho.lente.zero_inclinacao_grau.to_radians();
        let (sen_i, cos_i) = inclinacao.sin_cos();
        let (sen_g, cos_g) = a.torre[0].sin_cos();
        let c = &olho.no_corpo;
        Self {
            sen_i,
            cos_i,
            sen_g,
            cos_g,
            altura: c.eixo_altura_m + c.lente_frente_m * sen_i + c.lente_acima_m * cos_i,
            a_frente: c.lente_frente_m * cos_i - c.lente_acima_m * sen_i,
            pose: a.pose,
        }
    }

    /// Quanto o raio da linha `yn` desce por metro de profundidade: positivo abaixo do horizonte, zero nele.
    pub fn denominador(&self, yn: f64) -> f64 {
        yn * self.cos_i - self.sen_i
    }

    /// A profundidade que o chão tem na linha `yn`; nenhuma do horizonte para cima.
    pub fn chao(&self, yn: f64) -> Option<f64> {
        let denominador = self.denominador(yn);
        (denominador > 0.0).then(|| self.altura / denominador)
    }

    /// O que multiplica a inclinação da reta do inverso da profundidade contra `yn`, no chão, para dar a escala: no chão o
    /// inverso vale `escala * (yn * cos - sen) / altura`, e a reta sobe `escala * cos / altura` por unidade de `yn`.
    pub fn escala_por_inclinacao_da_reta(&self) -> f64 {
        self.altura / self.cos_i
    }

    /// O ponto visto em `[xn, yn]` à profundidade `z`, no corpo: `[x, y, altura acima do chão]`.
    pub fn no_corpo(&self, xn: f64, yn: f64, z: f64) -> [f64; 3] {
        let frente = self.a_frente + z * (self.cos_i + yn * self.sen_i);
        let esquerda = -z * xn;
        [
            self.cos_g * frente - self.sen_g * esquerda,
            self.sen_g * frente + self.cos_g * esquerda,
            self.altura - z * self.denominador(yn),
        ]
    }

    /// O mesmo ponto, na odometria: `[x, y, altura acima do chão]`.
    pub fn no_odom(&self, xn: f64, yn: f64, z: f64) -> [f64; 3] {
        let p = self.no_corpo(xn, yn, z);
        let xy = corpo_para_odom(self.pose, [p[0], p[1]]);
        [xy[0], xy[1], p[2]]
    }

    /// A volta: onde um ponto do corpo, `[x, y, altura]`, aparece: `[xn, yn, z]`. Nenhum lugar, se fica atrás da lente.
    pub fn da_lente(&self, p: [f64; 3]) -> Option<[f64; 3]> {
        let frente = self.cos_g * p[0] + self.sen_g * p[1] - self.a_frente;
        let esquerda = -self.sen_g * p[0] + self.cos_g * p[1];
        let cima = p[2] - self.altura;
        let z = frente * self.cos_i + cima * self.sen_i;
        (z > 0.0).then(|| {
            [
                -esquerda / z,
                (frente * self.sen_i - cima * self.cos_i) / z,
                z,
            ]
        })
    }
}
