//! A grade de profundidade que a estação entrega (a entrega `mapa`), e o número dela levado ao inverso da profundidade.
//!
//! O formato é o do leitor de profundidade da estação (no laboratório, `tools/hailo/servico/leitor_profundidade.cpp`): 16 bytes
//! de cabeçalho e a grade, tudo com o byte menor primeiro.
//!   0  u16  largura da grade (colunas)
//!   2  u16  altura da grade (linhas)
//!   4  u8   tipo do número: 0 = 8 bits, 1 = 16 bits, 2 = ponto flutuante de 32 bits
//!   5  u8   espécie: 1 = `fast_depth`, 2 = `depth_anything`, 3 = `scdepthv3`
//!   6  u16  zero
//!   8  f32  escala da quantização
//!  12  f32  zero da quantização
//!  16       a grade, linha por linha, de cima para baixo, nos bytes do próprio modelo
//! O valor de cada número é `(q - zero) * escala`.

pub const CABECALHO: usize = 16;

/// A espécie do número do `scdepthv3`: o de antes da conta `1 / (sigmoide(x) * 10 + 0,009)`. É a única que vira metros.
pub const ESPECIE_SCDEPTHV3: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Numero {
    U8,
    U16,
    F32,
}

impl Numero {
    fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 => 2,
            Self::F32 => 4,
        }
    }
}

/// Uma entrega lida: o cabeçalho e os bytes da grade, sem cópia.
#[derive(Debug, Clone, Copy)]
pub struct Grade<'a> {
    pub largura: usize,
    pub altura: usize,
    pub numero: Numero,
    pub especie: u8,
    pub escala: f32,
    pub zero: f32,
    dados: &'a [u8],
}

/// Lê a entrega. Recusa, pelo motivo, a que não fecha: a grade errada em silêncio viraria metros errados.
pub fn ler(carga: &[u8]) -> Result<Grade<'_>, String> {
    if carga.len() < CABECALHO {
        return Err(format!(
            "a grade tem {} bytes, e só o cabeçalho tem {CABECALHO}",
            carga.len()
        ));
    }
    let u16_em = |em: usize| usize::from(u16::from_le_bytes([carga[em], carga[em + 1]]));
    let f32_em =
        |em: usize| f32::from_le_bytes([carga[em], carga[em + 1], carga[em + 2], carga[em + 3]]);
    let numero = match carga[4] {
        0 => Numero::U8,
        1 => Numero::U16,
        2 => Numero::F32,
        outro => {
            return Err(format!(
                "a grade diz o tipo de número {outro}, e só há 0, 1 e 2"
            ));
        }
    };
    let (largura, altura) = (u16_em(0), u16_em(2));
    if largura == 0 || altura == 0 {
        return Err(format!("a grade diz {largura} por {altura}"));
    }
    let esperado = CABECALHO + largura * altura * numero.bytes();
    if carga.len() != esperado {
        return Err(format!(
            "a grade de {largura} por {altura} tem de ter {esperado} bytes, e vieram {}",
            carga.len()
        ));
    }
    let (escala, zero) = (f32_em(8), f32_em(12));
    if !(escala.is_finite() && zero.is_finite()) {
        return Err("a grade diz uma escala ou um zero da quantização que não é número".to_owned());
    }
    Ok(Grade {
        largura,
        altura,
        numero,
        especie: carga[5],
        escala,
        zero,
        dados: &carga[CABECALHO..],
    })
}

impl Grade<'_> {
    /// O número `k` da grade, como o modelo o escreveu, antes da quantização ser desfeita.
    fn cru(&self, k: usize) -> f32 {
        match self.numero {
            Numero::U8 => f32::from(self.dados[k]),
            Numero::U16 => f32::from(u16::from_le_bytes([
                self.dados[2 * k],
                self.dados[2 * k + 1],
            ])),
            Numero::F32 => f32::from_le_bytes([
                self.dados[4 * k],
                self.dados[4 * k + 1],
                self.dados[4 * k + 2],
                self.dados[4 * k + 3],
            ]),
        }
    }

    /// O valor do número `k`: `(q - zero) * escala`.
    pub fn valor(&self, k: usize) -> f32 {
        (self.cru(k) - self.zero) * self.escala
    }
}

/// O inverso da profundidade do `scdepthv3`, a menos de uma escala: `10 * sigmoide(x) + 0,009`. A conta é a da Hailo
/// (`depth_estimation_postprocessing.py:10-12`, guardada no laboratório em `fontes/hailo-profundidade/`).
pub fn inverso_do_scdepthv3(x: f32) -> f32 {
    10.0 / (1.0 + (-x).exp()) + 0.009
}

/// O inverso da profundidade de cada número de uma grade do `scdepthv3`. Com a grade em 8 ou em 16 bits, a conta é feita uma
/// vez por valor possível e guardada: a exponencial de 57.600 pontos, 15 vezes por segundo, pesaria no processador do robô.
#[derive(Debug, Default)]
pub struct Conversor {
    /// A quantização para a qual a tabela foi feita: o tipo, e os bits da escala e do zero.
    feita_para: Option<(Numero, u32, u32)>,
    tabela: Vec<f32>,
}

impl Conversor {
    pub fn preparar(&mut self, g: &Grade<'_>) {
        let chave = (g.numero, g.escala.to_bits(), g.zero.to_bits());
        if self.feita_para == Some(chave) {
            return;
        }
        let valores = match g.numero {
            Numero::U8 => 1usize << 8,
            Numero::U16 => 1usize << 16,
            Numero::F32 => 0,
        };
        self.tabela = (0..valores)
            .map(|q| inverso_do_scdepthv3((q as f32 - g.zero) * g.escala))
            .collect();
        self.feita_para = Some(chave);
    }

    /// O inverso da profundidade do número `k`. Só vale depois de `preparar` com esta grade.
    pub fn inverso(&self, g: &Grade<'_>, k: usize) -> f32 {
        match g.numero {
            Numero::U8 => self.tabela[usize::from(g.dados[k])],
            Numero::U16 => {
                self.tabela[usize::from(u16::from_le_bytes([g.dados[2 * k], g.dados[2 * k + 1]]))]
            }
            Numero::F32 => inverso_do_scdepthv3(g.valor(k)),
        }
    }
}
