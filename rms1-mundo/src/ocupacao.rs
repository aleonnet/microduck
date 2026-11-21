//! A ocupação do chão em volta do robô: células no referencial da odometria, cada uma não vista, livre ou ocupada, com a hora
//! da última vez.
//!
//! Cada ponto da grade, já em metros, é uma de três coisas:
//! - **chão**, quando a profundidade dele é a que o chão tem ali: a célula em que o raio para foi vista livre, e também
//!   o trecho entre dois pontos de chão seguidos da mesma coluna (longe, duas linhas seguidas caem a mais de uma célula uma da
//!   outra);
//! - **objeto**, quando está mais perto que o chão estaria (ou acima do horizonte): o ponto, levado ao chão, é candidato a
//!   ocupado, se a altura dele acima do chão fica entre a mínima e a máxima e ele é o primeiro de dois seguidos na coluna;
//! - **nada**, quando está mais longe que o chão (um reflexo, um buraco): não diz nada.
//!
//! Do cérebro do pato (`autod/src/obstacle.rs`), o que fez a memória dele confiável:
//! - um candidato só vale com vizinho: outro, de outra coluna, na mesma célula ou numa das oito em volta, neste quadro ou no
//!   anterior. Uma coluna sozinha com ruído não põe nada no mapa;
//! - "livre" não apaga o lembrado: a célula ocupada só volta a livre vista livre em `confirma_livre` quadros seguidos;
//! - o que não é visto de novo é esquecido depois de `lembra_s`.
//!
//! Uma célula só é vista livre pelo ponto de chão que cai nela: a que fica mais perto que o começo do chão visível, debaixo
//! da vista da câmera, nunca é limpa por um quadro que não a alcança.

use std::collections::{HashMap, HashSet};

use crate::Ajustes;
use crate::geometria::{Raios, Vista};
use crate::grade::{Conversor, Grade};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Estado {
    NaoVista = 0,
    Livre = 1,
    Ocupada = 2,
}

#[derive(Debug, Clone, Copy)]
struct Celula {
    /// De que célula do chão este lugar da janela é: a janela dá a volta, e o lugar de uma célula que saiu dela é reusado.
    de: [i32; 2],
    estado: Estado,
    /// Quando foi vista pela última vez no estado em que está.
    visto: f64,
    /// Estando ocupada: em quantos quadros seguidos foi vista livre, e o último deles.
    livres: u32,
    quadro_livre: u32,
}

const VAZIA: Celula = Celula {
    de: [i32::MIN, i32::MIN],
    estado: Estado::NaoVista,
    visto: f64::NEG_INFINITY,
    livres: 0,
    quadro_livre: 0,
};

/// O que um quadro acrescentou ao mapa.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Visto {
    pub livres: usize,
    pub ocupadas: usize,
}

pub struct Mapa {
    lado: f64,
    /// Quantas células a janela tem de lado.
    n: i32,
    celulas: Vec<Celula>,
    /// A célula em que o robô estava no último quadro: o meio da janela.
    centro: [i32; 2],
    quadro: u32,
    /// As células candidatas a ocupadas do quadro anterior.
    anteriores: HashSet<[i32; 2]>,
}

impl Mapa {
    pub fn novo(ajustes: &Ajustes) -> Self {
        let n = ((ajustes.janela_m / ajustes.celula_m).ceil() as i32).max(2);
        Self {
            lado: ajustes.celula_m,
            n,
            celulas: vec![VAZIA; (n * n) as usize],
            centro: [0, 0],
            quadro: 0,
            anteriores: HashSet::new(),
        }
    }

    pub fn lado(&self) -> f64 {
        self.lado
    }

    /// A célula em que um ponto da odometria cai.
    pub fn celula_de(&self, p: [f64; 2]) -> [i32; 2] {
        [
            (p[0] / self.lado).floor() as i32,
            (p[1] / self.lado).floor() as i32,
        ]
    }

    /// O meio de uma célula, na odometria.
    pub fn meio_de(&self, c: [i32; 2]) -> [f64; 2] {
        [
            (f64::from(c[0]) + 0.5) * self.lado,
            (f64::from(c[1]) + 0.5) * self.lado,
        ]
    }

    fn lugar(&self, c: [i32; 2]) -> usize {
        (c[0].rem_euclid(self.n) + self.n * c[1].rem_euclid(self.n)) as usize
    }

    fn na_janela(&self, c: [i32; 2]) -> bool {
        (c[0] - self.centro[0]).abs() < self.n / 2 && (c[1] - self.centro[1]).abs() < self.n / 2
    }

    /// O lugar da célula, pronto para escrever: o que era de outra célula é esvaziado.
    fn pegar(&mut self, c: [i32; 2]) -> &mut Celula {
        let lugar = self.lugar(c);
        let cel = &mut self.celulas[lugar];
        if cel.de != c {
            *cel = Celula { de: c, ..VAZIA };
        }
        cel
    }

    /// O estado de uma célula no instante `t`, e há quanto tempo foi vista.
    pub fn estado(&self, c: [i32; 2], t: f64, lembra_s: f64) -> (Estado, f64) {
        let cel = &self.celulas[self.lugar(c)];
        let idade = t - cel.visto;
        if cel.de != c || !self.na_janela(c) || cel.estado == Estado::NaoVista || idade > lembra_s {
            return (Estado::NaoVista, f64::INFINITY);
        }
        (cel.estado, idade.max(0.0))
    }

    /// As células vistas e ainda lembradas no instante `t`: a célula, o estado e a idade.
    pub fn vivas(
        &self,
        t: f64,
        lembra_s: f64,
    ) -> impl Iterator<Item = ([i32; 2], Estado, f64)> + '_ {
        self.celulas.iter().filter_map(move |cel| {
            let idade = t - cel.visto;
            (cel.estado != Estado::NaoVista && idade <= lembra_s && self.na_janela(cel.de))
                .then(|| (cel.de, cel.estado, idade.max(0.0)))
        })
    }

    /// Acrescenta ao mapa o que a grade mostra, já com a escala dela, vista de onde a câmera estava no instante do quadro.
    #[allow(clippy::too_many_arguments)]
    pub fn observar(
        &mut self,
        g: &Grade<'_>,
        conversor: &Conversor,
        vista: &Vista,
        raios: &Raios,
        escala: f64,
        t: f64,
        ajustes: &Ajustes,
    ) -> Visto {
        self.quadro = self.quadro.wrapping_add(1);
        self.centro = self.celula_de([vista.pose[0], vista.pose[1]]);
        let alcance2 = ajustes.alcance_m * ajustes.alcance_m;
        let meia = self.lado / 2.0;
        let mut livres: Vec<[i32; 2]> = Vec::new();
        // Cada candidata a ocupada, com a coluna da grade em que apareceu.
        let mut candidatas: Vec<([i32; 2], usize)> = Vec::new();
        for i in 0..g.largura {
            let xn = raios.xn[i];
            // O ponto de chão da linha de baixo, e a candidata da linha de baixo: o que liga dois pontos seguidos da coluna.
            let mut chao_de_baixo: Option<[f64; 2]> = None;
            let mut objeto_de_baixo: Option<[i32; 2]> = None;
            for j in (0..g.altura).rev() {
                let yn = raios.yn[j];
                // O ponto que olha para o próprio robô (o cano do lançador) não diz nada do mundo, como o que veio sem
                // profundidade.
                let k = j * g.largura + i;
                let u = f64::from(conversor.inverso(g, k));
                if u <= 0.0 || raios.tapado(k) {
                    (chao_de_baixo, objeto_de_baixo) = (None, None);
                    continue;
                }
                let z = escala / u;
                if let Some(z_chao) = vista.chao(yn) {
                    let razao = z / z_chao;
                    let e_chao = (razao - 1.0).abs() <= ajustes.tolerancia;
                    if e_chao {
                        objeto_de_baixo = None;
                        // Onde o raio para: no chão, ou antes dele, se o ponto está um pouco mais perto (a base de um
                        // objeto, dentro da tolerância). Nunca depois: o raio não passa do chão, e marcar além pintaria de
                        // livre o que fica atrás da base do objeto.
                        let ate = z.min(z_chao);
                        let p = vista.no_corpo(xn, yn, ate);
                        if p[0] * p[0] + p[1] * p[1] > alcance2 {
                            chao_de_baixo = None;
                            continue;
                        }
                        let aqui = vista.no_odom(xn, yn, ate);
                        let aqui = [aqui[0], aqui[1]];
                        match chao_de_baixo {
                            // Entre dois pontos de chão seguidos, o chão: a cada meia célula do trecho.
                            Some(antes) => {
                                let (dx, dy) = (aqui[0] - antes[0], aqui[1] - antes[1]);
                                let passos = ((dx.hypot(dy) / meia).ceil() as usize).max(1);
                                for k in 1..=passos {
                                    let f = k as f64 / passos as f64;
                                    let c = self.celula_de([antes[0] + f * dx, antes[1] + f * dy]);
                                    if livres.last() != Some(&c) {
                                        livres.push(c);
                                    }
                                }
                            }
                            None => livres.push(self.celula_de(aqui)),
                        }
                        chao_de_baixo = Some(aqui);
                        continue;
                    }
                    if razao > 1.0 {
                        // Mais longe que o chão: não é chão nem objeto.
                        (chao_de_baixo, objeto_de_baixo) = (None, None);
                        continue;
                    }
                }
                chao_de_baixo = None;
                let p = vista.no_corpo(xn, yn, z);
                let no_caminho = p[2] > ajustes.altura_minima_m
                    && p[2] < ajustes.altura_maxima_m
                    && p[0] * p[0] + p[1] * p[1] <= alcance2;
                if !no_caminho {
                    objeto_de_baixo = None;
                    continue;
                }
                let aqui = vista.no_odom(xn, yn, z);
                let c = self.celula_de([aqui[0], aqui[1]]);
                // O primeiro de dois pontos seguidos: o de baixo entra quando o de cima o confirma.
                if let Some(de_baixo) = objeto_de_baixo {
                    candidatas.push((de_baixo, i));
                }
                objeto_de_baixo = Some(c);
            }
        }

        // A coluna mais à esquerda e a mais à direita em que cada célula foi candidata.
        let mut colunas: HashMap<[i32; 2], (usize, usize)> = HashMap::new();
        for &(c, coluna) in &candidatas {
            colunas
                .entry(c)
                .and_modify(|(de, ate)| {
                    *de = (*de).min(coluna);
                    *ate = (*ate).max(coluna);
                })
                .or_insert((coluna, coluna));
        }
        let em_volta = |c: [i32; 2]| {
            (-1..=1).flat_map(move |dx| (-1..=1).map(move |dy| [c[0] + dx, c[1] + dy]))
        };
        let mut confirmadas: Vec<[i32; 2]> = Vec::new();
        for (&c, &(de, ate)) in &colunas {
            // Vizinho no mesmo quadro: na célula e nas oito em volta, candidatas de mais de uma coluna.
            let (mut primeira, mut ultima) = (de, ate);
            for v in em_volta(c) {
                if let Some(&(d, a)) = colunas.get(&v) {
                    primeira = primeira.min(d);
                    ultima = ultima.max(a);
                }
            }
            let neste = primeira != ultima;
            let no_anterior = em_volta(c).any(|v| self.anteriores.contains(&v));
            if neste || no_anterior {
                confirmadas.push(c);
            }
        }
        self.anteriores = colunas.into_keys().collect();

        let mut visto = Visto::default();
        let (quadro, confirma, lembra_s) = (self.quadro, ajustes.confirma_livre, ajustes.lembra_s);
        for c in livres {
            if !self.na_janela(c) {
                continue;
            }
            let cel = self.pegar(c);
            // Muitos pontos caem na mesma célula: ela conta uma vez por quadro.
            if cel.quadro_livre == quadro {
                continue;
            }
            let seguido = cel.quadro_livre.wrapping_add(1) == quadro;
            cel.quadro_livre = quadro;
            if cel.estado == Estado::Ocupada && t - cel.visto <= lembra_s {
                // "Livre" não apaga o lembrado: só depois de visto livre em quadros seguidos.
                cel.livres = if seguido { cel.livres + 1 } else { 1 };
                if cel.livres < confirma {
                    continue;
                }
            }
            cel.estado = Estado::Livre;
            cel.visto = t;
            cel.livres = 0;
            visto.livres += 1;
        }
        for c in confirmadas {
            if !self.na_janela(c) {
                continue;
            }
            let cel = self.pegar(c);
            cel.estado = Estado::Ocupada;
            cel.visto = t;
            cel.livres = 0;
            visto.ocupadas += 1;
        }
        visto
    }
}
