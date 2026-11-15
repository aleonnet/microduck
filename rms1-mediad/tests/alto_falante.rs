//! O alto-falante: os critérios 8 e 9 do plano, com uma saída de mentira que anota tudo o que o tocador faz.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rms1_mediad::alto_falante::{Saida, Tocador, Tocando, WHEEE_VALE};
use rms1_mediad::trava::Trava;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Fato {
    Tocou(String),
    Cortou(String),
    AbriuFluxo,
    CortouFluxo,
}

#[derive(Clone, Default)]
struct Anotacoes {
    fatos: Arc<Mutex<Vec<Fato>>>,
    /// O que foi escrito no fluxo do `wheee`.
    fluxo: Arc<Mutex<Vec<u8>>>,
    /// Quantos sons estão vivos ao mesmo tempo, e o máximo que já estiveram.
    vivos: Arc<Mutex<(u32, u32)>>,
}

impl Anotacoes {
    fn fatos(&self) -> Vec<Fato> {
        self.fatos.lock().unwrap().clone()
    }
    fn entrou(&self) {
        let mut v = self.vivos.lock().unwrap();
        v.0 += 1;
        v.1 = v.1.max(v.0);
    }
    fn saiu(&self) {
        self.vivos.lock().unwrap().0 -= 1;
    }
}

/// Um som de mentira: toca até ser cortado (os arquivos do teste nunca terminam sozinhos, salvo o fluxo fechado).
struct SomFalso {
    nome: Option<String>,
    a: Anotacoes,
    vivo: bool,
    fechado: Arc<AtomicBool>,
}

impl Tocando for SomFalso {
    fn terminou(&mut self) -> bool {
        if self.vivo && self.fechado.load(Ordering::Relaxed) {
            self.vivo = false;
            self.a.saiu();
        }
        !self.vivo
    }
    fn cortar(&mut self) {
        if self.vivo {
            self.vivo = false;
            self.a.saiu();
            self.a.fatos.lock().unwrap().push(match &self.nome {
                Some(n) => Fato::Cortou(n.clone()),
                None => Fato::CortouFluxo,
            });
        }
        // O fluxo cortado recusa o que ainda for escrito, como o `aplay` morto.
        self.fechado.store(true, Ordering::Relaxed);
    }
}

/// O lado de escrita do fluxo: guarda os bytes, e avisa quando quem escreve termina.
struct Escrita {
    a: Anotacoes,
    fechado: Arc<AtomicBool>,
}

impl Write for Escrita {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if self.fechado.load(Ordering::Relaxed) {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        self.a.fluxo.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for Escrita {
    fn drop(&mut self) {
        // Quem escreve acabou: o som termina sozinho.
        self.fechado.store(true, Ordering::Relaxed);
    }
}

struct SaidaFalsa(Anotacoes);

impl Saida for SaidaFalsa {
    fn tocar_arquivo(&mut self, wav: &Path) -> std::io::Result<Box<dyn Tocando>> {
        let nome = wav
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        self.0.fatos.lock().unwrap().push(Fato::Tocou(nome.clone()));
        self.0.entrou();
        Ok(Box::new(SomFalso {
            nome: Some(nome),
            a: self.0.clone(),
            vivo: true,
            fechado: Arc::new(AtomicBool::new(false)),
        }))
    }
    fn abrir_fluxo(
        &mut self,
        _hz: u32,
    ) -> std::io::Result<(Box<dyn Write + Send>, Box<dyn Tocando>)> {
        self.0.fatos.lock().unwrap().push(Fato::AbriuFluxo);
        self.0.entrou();
        let fechado = Arc::new(AtomicBool::new(false));
        Ok((
            Box::new(Escrita {
                a: self.0.clone(),
                fechado: fechado.clone(),
            }),
            Box::new(SomFalso {
                nome: None,
                a: self.0.clone(),
                vivo: true,
                fechado,
            }),
        ))
    }
}

/// Um `.wav` mono de 16 bits a 48 kHz com `n` amostras de valor `v`.
fn wav(caminho: &Path, v: i16, n: usize) {
    std::fs::create_dir_all(caminho.parent().unwrap()).unwrap();
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(caminho, spec).unwrap();
    for _ in 0..n {
        w.write_sample(v).unwrap();
    }
    w.finalize().unwrap();
}

/// Um banco com `chirp`, `greet` e o `wheee` em três partes, cada parte com um valor só: dá para saber, pelos bytes do
/// fluxo, que parte foi tocada.
fn banco(pasta: &Path) {
    wav(&pasta.join("chirp/chirp_a.wav"), 1, 100);
    wav(&pasta.join("greet/greet_a.wav"), 2, 100);
    wav(&pasta.join("wheee/wheee_start_a.wav"), 0x1111, 2400);
    wav(&pasta.join("wheee/wheee_loop_a.wav"), 0x2222, 2400);
    wav(&pasta.join("wheee/wheee_end_a.wav"), 0x3333, 2400);
}

fn montar() -> (tempfile::TempDir, Anotacoes, Trava, Tocador) {
    let pasta = tempfile::tempdir().unwrap();
    banco(pasta.path());
    let a = Anotacoes::default();
    let trava = Trava::nova(Duration::from_millis(800));
    let tocador = Tocador::novo(
        pasta.path().to_owned(),
        Box::new(SaidaFalsa(a.clone())),
        trava.clone(),
    );
    (pasta, a, trava, tocador)
}

/// Roda os tiques do tocador até `cond` valer, com prazo.
fn ate(tocador: &mut Tocador, mut cond: impl FnMut() -> bool, o_que: &str) {
    let limite = Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(Instant::now() < limite, "não aconteceu em 5 s: {o_que}");
        tocador.tique(Instant::now());
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn contem(bytes: &[u8], valor: i16) -> bool {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .any(|c| i16::from_le_bytes(*c) == valor)
}

#[test]
fn um_som_por_vez() {
    let (_pasta, a, trava, mut tocador) = montar();
    let t = Instant::now();
    tocador.tocar("chirp", t);
    assert!(trava.vale(t), "com o som tocando, a trava vale");
    tocador.tocar("greet", t);
    assert_eq!(
        a.fatos(),
        [
            Fato::Tocou("chirp".into()),
            Fato::Cortou("chirp".into()),
            Fato::Tocou("greet".into())
        ]
    );
    assert_eq!(
        a.vivos.lock().unwrap().1,
        1,
        "dois sons tocaram ao mesmo tempo"
    );

    // Durante o `wheee`, o avulso é descartado: não corta o passeio nem toca depois.
    tocador.wheee(true, Instant::now());
    let antes = a.fatos().len();
    tocador.tocar("chirp", Instant::now());
    assert_eq!(
        a.fatos().len(),
        antes,
        "o avulso mexeu na saída durante o wheee"
    );
    assert_eq!(a.fatos().last(), Some(&Fato::AbriuFluxo));
    assert_eq!(a.vivos.lock().unwrap().1, 1);
    tocador.wheee(false, Instant::now());
}

#[test]
fn wheee_pousa_ou_corta() {
    // Sem renovação por 300 ms: pousa, tocando o fim.
    let (_pasta, a, trava, mut tocador) = montar();
    tocador.wheee(true, Instant::now());
    ate(
        &mut tocador,
        || contem(&a.fluxo.lock().unwrap(), 0x2222),
        "o laço do wheee",
    );
    assert!(
        !contem(&a.fluxo.lock().unwrap(), 0x3333),
        "o fim tocou com o pedido ainda valendo"
    );
    std::thread::sleep(WHEEE_VALE);
    ate(
        &mut tocador,
        || contem(&a.fluxo.lock().unwrap(), 0x3333),
        "o fim do wheee depois de 300 ms sem renovação",
    );
    ate(
        &mut tocador,
        || !tocador_tocando(&a),
        "o som terminar depois do fim",
    );
    assert!(
        !a.fatos().contains(&Fato::CortouFluxo),
        "pousar não é cortar"
    );
    tocador.tique(Instant::now());
    assert!(!tocador.tocando());
    assert!(
        trava.vale(Instant::now()),
        "logo depois do som, a janela da trava ainda vale"
    );

    // `hold: false`: corta onde está, sem o fim.
    let (_pasta, a, _trava, mut tocador) = montar();
    tocador.wheee(true, Instant::now());
    ate(
        &mut tocador,
        || contem(&a.fluxo.lock().unwrap(), 0x2222),
        "o laço do wheee",
    );
    tocador.wheee(false, Instant::now());
    assert_eq!(a.fatos().last(), Some(&Fato::CortouFluxo));
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        !contem(&a.fluxo.lock().unwrap(), 0x3333),
        "o corte tocou o fim"
    );
    assert!(!tocador.tocando());

    // Renovado a cada 100 ms, o passeio segue por mais que 300 ms.
    let (_pasta, a, _trava, mut tocador) = montar();
    tocador.wheee(true, Instant::now());
    for _ in 0..6 {
        std::thread::sleep(Duration::from_millis(100));
        tocador.wheee(true, Instant::now());
        tocador.tique(Instant::now());
    }
    assert!(
        !contem(&a.fluxo.lock().unwrap(), 0x3333),
        "o wheee pousou com o pedido sendo renovado"
    );
    assert_eq!(
        a.fatos(),
        [Fato::AbriuFluxo],
        "o wheee recomeçou a cada renovação"
    );
    tocador.wheee(false, Instant::now());
}

fn tocador_tocando(a: &Anotacoes) -> bool {
    a.vivos.lock().unwrap().0 > 0
}
