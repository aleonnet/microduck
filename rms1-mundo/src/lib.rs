//! A camada de mundo do S1: da grade de profundidade que a estação entrega ao chão em volta do robô, em metros.
//!
//! Quem chama (o `rms1-mediad`) dá três coisas: a câmera, cada amostra da torre e da pose que o `rms1d` publica, e cada grade
//! com a hora em que o quadro dela foi tirado. O mundo casa a grade com a amostra daquele instante, acha a escala pelo chão,
//! marca no mapa o que é livre e o que é ocupado, e responde, para qualquer instante, a folga por direção em volta do corpo,
//! o obstáculo à frente e o estado da percepção. "A percepção falhou" e "não vi nada" são estados diferentes.
//!
//! Puro: sem soquete e sem relógio próprio. A hora é sempre a de quem chama, em segundos de um relógio que não volta. Os
//! testes o dirigem com cenas desenhadas, e uma sessão gravada no robô passa pelo mesmo caminho (`gravacao`).

pub mod escala;
pub mod folga;
pub mod geometria;
pub mod grade;
pub mod gravacao;
pub mod historia;
pub mod ocupacao;

use serde_json::{Value, json};

pub use folga::{Corpo, Obstaculo, Setor};
pub use geometria::{Olho, Raios, Vista};
pub use historia::Amostra;
pub use ocupacao::Estado;
/// Os limiares da camada: `[midia.mundo]`, de `rms1d.toml`.
pub use rms1d_params::MidiaMundo as Ajustes;

/// O leitor da estação cuja grade vira metros.
pub const LEITOR_COM_METROS: &str = "scdepthv3";

/// De onde veio a escala da última grade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Escala {
    /// Achada no chão do próprio quadro.
    Medida,
    /// A dos quadros de antes: o chão saiu da faixa de baixo deste.
    Lembrada,
}

/// O estado da percepção, como `s1.world` o diz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Percepcao {
    Ok,
    /// Sem grade há mais de `velho_s`.
    Velha,
    /// A estação disse que a função falhou, ou recusou o quadro.
    Falhou,
    /// A grade chegou, e não há escala: o chão não foi reconhecido, e nenhuma escala de antes ainda vale.
    SemChao,
    /// O leitor ligado na estação não dá metros.
    SemMetros,
    /// Sem a torre e a pose do instante do quadro: o `rms1d` não está publicando.
    SemPose,
    /// A câmera do robô ainda não foi medida.
    SemCamera,
}

impl Percepcao {
    pub fn nome(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Velha => "stale",
            Self::Falhou => "failed",
            Self::SemChao => "no_floor",
            Self::SemMetros => "no_metres",
            Self::SemPose => "no_pose",
            Self::SemCamera => "no_camera",
        }
    }
}

/// O que uma grade deu, para quem a entregou medir e contar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tratada {
    pub percepcao: Percepcao,
    pub escala: Option<Escala>,
    /// A escala do próprio quadro, com a qualidade dela, quando ele a deu.
    pub do_quadro: Option<escala::DoQuadro>,
    /// O exame da faixa de baixo do quadro, valha a escala dele ou não; nada, se a conta nem chegou a ele (sem a câmera, sem
    /// metros, sem a pose do instante).
    pub exame: Option<escala::Exame>,
    /// A escala que levou esta grade aos metros: a do quadro, ou a lembrada.
    pub valor: Option<f64>,
    pub visto: ocupacao::Visto,
}

/// O mundo num instante, no corpo daquele instante.
#[derive(Debug, Clone, PartialEq)]
pub struct Agora {
    /// Há quanto tempo o quadro da última grade foi tirado, em milissegundos.
    pub idade_do_quadro_ms: Option<f64>,
    pub percepcao: Percepcao,
    pub motivo: Option<String>,
    pub escala: Option<Escala>,
    pub obstaculo: Option<Obstaculo>,
    pub folga: Vec<Setor>,
    /// O campo de visão da lente, em graus: o horizontal e o vertical.
    pub lente: Option<[f64; 2]>,
}

/// O número que o JSON aceita: o infinito e o que não é número viram `null`.
fn numero(v: f64, casas: i32) -> Value {
    if !v.is_finite() {
        return Value::Null;
    }
    let p = 10f64.powi(casas);
    json!((v * p).round() / p)
}

impl Agora {
    /// Os parâmetros de `s1.world`. A folga é uma lista de `[rumo em graus, metros, estado, idade em segundos]`, com o estado
    /// 0 (não visto), 1 (livre até ali) ou 2 (ocupado ali); no obstáculo, `ahead_m` nulo é o corredor livre.
    pub fn linha(&self) -> Value {
        let folga: Vec<Value> = self
            .folga
            .iter()
            .map(|s| {
                json!([
                    numero(s.rumo_grau, 1),
                    numero(s.livre_m, 3),
                    s.estado as u8,
                    numero(s.idade_s, 2)
                ])
            })
            .collect();
        json!({
            "frame": "body",
            "frame_age_ms": self.idade_do_quadro_ms.map(|i| numero(i, 0)),
            "lens": self.lente.map(|[h, v]| json!({"hfov_deg": numero(h, 2), "vfov_deg": numero(v, 2)})),
            "perception": {
                "state": self.percepcao.nome(),
                "reason": self.motivo,
                "scale": self.escala.map(|e| match e { Escala::Medida => "measured", Escala::Lembrada => "remembered" }),
            },
            "obstacle": self.obstaculo.map(|o| json!({"ahead_m": numero(o.ahead_m, 3), "away": o.away, "age_s": numero(o.age_s, 2)})),
            "clearance": folga,
        })
    }
}

/// O que a última grade deu.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Ultima {
    /// A hora em que o quadro dela foi tirado.
    t: f64,
    percepcao: Percepcao,
    escala: Option<Escala>,
}

pub struct Mundo {
    olho: Option<Olho>,
    corpo: Corpo,
    ajustes: Ajustes,
    historia: historia::Historia,
    mapa: ocupacao::Mapa,
    escalas: escala::Memoria,
    conversor: grade::Conversor,
    raios: Option<Raios>,
    ultima: Option<Ultima>,
    /// A falha que a estação disse, com a hora.
    falha: Option<(f64, String)>,
    gravador: Option<gravacao::Gravador>,
}

impl Mundo {
    /// Sem a câmera (`olho`), o mundo só diz que ela falta. Com `[midia.mundo] gravar`, a sessão é gravada desde já; se o
    /// arquivo não abre, o erro volta, e nada é criado.
    pub fn novo(olho: Option<Olho>, corpo: Corpo, ajustes: Ajustes) -> std::io::Result<Self> {
        let gravador = if ajustes.gravar.is_empty() {
            None
        } else {
            Some(gravacao::Gravador::criar(
                std::path::Path::new(&ajustes.gravar),
                olho.as_ref(),
                &corpo,
                ajustes.gravar_limite_mb,
            )?)
        };
        Ok(Self {
            mapa: ocupacao::Mapa::novo(&ajustes),
            olho,
            corpo,
            ajustes,
            historia: historia::Historia::default(),
            escalas: escala::Memoria::default(),
            conversor: grade::Conversor::default(),
            raios: None,
            ultima: None,
            falha: None,
            gravador,
        })
    }

    pub fn ajustes(&self) -> &Ajustes {
        &self.ajustes
    }

    /// Uma amostra da torre e da pose, na hora em que chegou.
    pub fn amostra(&mut self, a: Amostra) {
        if let Some(g) = &mut self.gravador {
            g.amostra(&a);
        }
        self.historia.guardar(a);
    }

    /// A estação disse que a função da grade falhou neste quadro, ou recusou o quadro: fica dito até a grade seguinte.
    pub fn falha(&mut self, t: f64, motivo: &str) {
        if let Some(g) = &mut self.gravador {
            g.falha(t, motivo);
        }
        self.falha = Some((t, motivo.to_owned()));
    }

    /// Uma grade, do leitor que a estação anunciou, com a hora em que o quadro dela foi tirado. O erro é a grade que não se
    /// lê: quem chama a conta como erro, e o mundo fica como estava.
    pub fn grade(&mut self, carga: &[u8], leitor: &str, t_captura: f64) -> Result<Tratada, String> {
        let g = grade::ler(carga)?;
        if let Some(gravador) = &mut self.gravador {
            gravador.grade(t_captura, leitor, carga);
        }
        let mut tratada = Tratada {
            percepcao: Percepcao::Ok,
            escala: None,
            do_quadro: None,
            exame: None,
            valor: None,
            visto: ocupacao::Visto::default(),
        };
        tratada.percepcao = 'conta: {
            let Some(olho) = self.olho else {
                break 'conta Percepcao::SemCamera;
            };
            if leitor != LEITOR_COM_METROS || g.especie != grade::ESPECIE_SCDEPTHV3 {
                break 'conta Percepcao::SemMetros;
            }
            // O quadro é mais velho do que a hora de chegada diz, pelo atraso medido da câmera.
            let instante = t_captura - olho.lente.atraso_ms / 1000.0;
            let vale = self.ajustes.torre_vale_ms as f64 / 1000.0;
            let Some(amostra) = self
                .historia
                .mais_perto(instante)
                .filter(|a| (a.t - instante).abs() <= vale)
            else {
                break 'conta Percepcao::SemPose;
            };
            let vista = Vista::nova(&olho, &amostra);
            self.conversor.preparar(&g);
            if !self
                .raios
                .as_ref()
                .is_some_and(|r| r.serve(g.largura, g.altura))
            {
                self.raios = Some(Raios::da_grade(&olho, g.largura, g.altura));
            }
            let raios = self.raios.as_ref().expect("os raios desta grade");
            let exame = escala::examinar(&g, &self.conversor, &vista, raios, &self.ajustes);
            tratada.exame = Some(exame);
            tratada.do_quadro = exame.aceito();
            let valor = match tratada.do_quadro {
                Some(e) => {
                    self.escalas.guardar(t_captura, e.valor);
                    tratada.escala = Some(Escala::Medida);
                    e.valor
                }
                None => match self
                    .escalas
                    .lembrada(t_captura, self.ajustes.escala_vale_ms)
                {
                    Some(lembrada) => {
                        tratada.escala = Some(Escala::Lembrada);
                        lembrada
                    }
                    None => break 'conta Percepcao::SemChao,
                },
            };
            tratada.valor = Some(valor);
            tratada.visto = self.mapa.observar(
                &g,
                &self.conversor,
                &vista,
                raios,
                valor,
                t_captura,
                &self.ajustes,
            );
            Percepcao::Ok
        };
        self.ultima = Some(Ultima {
            t: t_captura,
            percepcao: tratada.percepcao,
            escala: tratada.escala,
        });
        Ok(tratada)
    }

    /// O mundo no instante `t`, no corpo da amostra mais nova.
    pub fn agora(&self, t: f64) -> Agora {
        let vale = self.ajustes.torre_vale_ms as f64 / 1000.0;
        let amostra = self.historia.ultima();
        let (folga, obstaculo) = match amostra {
            Some(a) => folga::do_mapa(&self.mapa, a.pose, t, &self.corpo, &self.ajustes),
            None => (Vec::new(), None),
        };
        let falhou = self
            .falha
            .as_ref()
            .filter(|(quando, _)| self.ultima.is_none_or(|u| *quando >= u.t));
        let (percepcao, motivo) = if self.olho.is_none() {
            (Percepcao::SemCamera, None)
        } else if !amostra.is_some_and(|a| t - a.t <= vale) {
            (Percepcao::SemPose, None)
        } else if let Some((quando, motivo)) = falhou
            && t - quando <= self.ajustes.velho_s
        {
            (Percepcao::Falhou, Some(motivo.clone()))
        } else {
            match self.ultima {
                Some(u) if t - u.t <= self.ajustes.velho_s => (u.percepcao, None),
                _ => (Percepcao::Velha, None),
            }
        };
        Agora {
            idade_do_quadro_ms: self.ultima.map(|u| (t - u.t) * 1000.0),
            percepcao,
            motivo,
            escala: self
                .ultima
                .filter(|_| percepcao == Percepcao::Ok)
                .and_then(|u| u.escala),
            obstaculo,
            folga,
            lente: self.olho.map(|o| o.campo_grau()),
        }
    }

    /// Os parâmetros de `s1.world.grid`: as células vistas e ainda lembradas, na odometria, com a pose da amostra mais nova
    /// (`[x, y, rumo em graus]`). Cada célula é `[ix, iy, estado, idade em segundos]`, e o canto dela fica em
    /// `(ix * cell_m, iy * cell_m)`.
    pub fn linha_da_grade(&self, t: f64) -> Value {
        let celulas: Vec<Value> = self
            .mapa
            .vivas(t, self.ajustes.lembra_s)
            .map(|(c, estado, idade)| json!([c[0], c[1], estado as u8, numero(idade, 2)]))
            .collect();
        json!({
            "frame": "odom",
            "pose": self.historia.ultima().map(|a| json!([numero(a.pose[0], 4), numero(a.pose[1], 4), numero(a.pose[2].to_degrees(), 2)])),
            "cell_m": self.mapa.lado(),
            "cells": celulas,
        })
    }

    /// As células vistas e ainda lembradas no instante `t`: a célula, o estado e a idade.
    pub fn celulas(&self, t: f64) -> Vec<([i32; 2], Estado, f64)> {
        self.mapa.vivas(t, self.ajustes.lembra_s).collect()
    }

    /// O estado da célula em que o ponto da odometria cai.
    pub fn estado_em(&self, ponto: [f64; 2], t: f64) -> Estado {
        self.mapa
            .estado(self.mapa.celula_de(ponto), t, self.ajustes.lembra_s)
            .0
    }

    /// Se a gravação segue no ar: falso sem `gravar`, ou depois de ela parar (o disco, ou o limite do arquivo).
    pub fn gravando(&self) -> bool {
        self.gravador
            .as_ref()
            .is_some_and(gravacao::Gravador::no_ar)
    }
}
