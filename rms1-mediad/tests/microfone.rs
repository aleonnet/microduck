//! O microfone e a trava: os critérios 5 a 7 do plano.

use std::time::{Duration, Instant};

use rms1_mediad::microfone::{self, BLOCO, Ouvido, Ouvinte};
use rms1_mediad::trava::Trava;

const HZ: usize = 16_000;

/// Ruído determinístico de amplitude `a`, por `s` segundos: um gerador congruente, para o teste dar sempre o mesmo.
fn ruido(a: f32, s: f32, semente: &mut u32) -> Vec<f32> {
    (0..(s * HZ as f32) as usize)
        .map(|_| {
            *semente = semente.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            a * ((*semente >> 8) as f32 / 8_388_608.0 - 1.0)
        })
        .collect()
}

/// Uma sala quieta, uma palma (0,1 s) e, depois de um silêncio, uma fala (1 s).
fn palma_e_fala() -> Vec<f32> {
    let mut s = 7u32;
    let mut a = ruido(0.001, 3.0, &mut s);
    a.extend(ruido(0.3, 0.1, &mut s));
    a.extend(ruido(0.001, 2.0, &mut s));
    a.extend(ruido(0.2, 1.0, &mut s));
    a.extend(ruido(0.001, 1.0, &mut s));
    a
}

/// Passa as amostras pelo ouvinte em blocos, com o relógio andando no ritmo delas a partir de `t0`.
fn ouvir(ouvinte: &mut Ouvinte, amostras: &[f32], t0: Instant) -> Vec<(Ouvido, Duration)> {
    let mut saiu = Vec::new();
    for (i, bloco) in amostras.chunks(BLOCO).enumerate() {
        let quando = Duration::from_secs_f64((i * BLOCO) as f64 / HZ as f64);
        for o in ouvinte.empurrar(bloco, t0 + quando) {
            saiu.push((o, quando));
        }
    }
    saiu
}

#[test]
fn arquivo_com_palma_e_fala_da_noise_e_voice() {
    // Pelo caminho do serviço: um `.wav` no disco, lido pela fonte de arquivo.
    let pasta = tempfile::tempdir().unwrap();
    let caminho = pasta.path().join("palma_e_fala.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: HZ as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&caminho, spec).unwrap();
    for a in palma_e_fala() {
        w.write_sample((a * 32767.0) as i16).unwrap();
    }
    w.finalize().unwrap();
    let amostras = microfone::ler_arquivo(caminho.to_str().unwrap()).unwrap();
    assert_eq!(amostras.len(), palma_e_fala().len());

    let mut ouvinte = Ouvinte::novo(Trava::nova(Duration::from_millis(800)));
    let saiu: Vec<Ouvido> = ouvir(&mut ouvinte, &amostras, Instant::now())
        .into_iter()
        .map(|(o, _)| o)
        .collect();
    assert_eq!(
        saiu,
        [Ouvido::Barulho, Ouvido::Voz],
        "uma palma e depois uma fala, nessa ordem"
    );
    assert_eq!(
        (Ouvido::Barulho.nome(), Ouvido::Voz.nome()),
        ("noise", "voice")
    );
}

#[test]
fn o_proprio_som_e_descartado() {
    let trava = Trava::nova(Duration::from_millis(800));
    let mut ouvinte = Ouvinte::novo(trava.clone());
    let t0 = Instant::now();
    // O alto-falante toca durante o áudio inteiro: nada do que o microfone ouve pode sair.
    trava.comecou();
    assert!(
        ouvir(&mut ouvinte, &palma_e_fala(), t0).is_empty(),
        "saiu evento com o alto-falante tocando"
    );
    // E parou há menos que a janela: ainda descarta.
    let fim = t0 + Duration::from_secs(8);
    trava.terminou(fim);
    assert!(
        trava.vale(fim + Duration::from_millis(799)),
        "a janela depois do som"
    );
}

#[test]
fn depois_da_trava_volta_a_ouvir() {
    let trava = Trava::nova(Duration::from_millis(800));
    let mut ouvinte = Ouvinte::novo(trava.clone());
    let t0 = Instant::now();
    trava.comecou();
    trava.terminou(t0);
    assert!(trava.vale(t0 + Duration::from_millis(100)));
    assert!(
        !trava.vale(t0 + Duration::from_millis(801)),
        "passada a janela, a trava solta"
    );
    // O áudio começa 1 s depois do fim do som: a palma, aos 3 s, e a fala saem.
    let saiu = ouvir(&mut ouvinte, &palma_e_fala(), t0 + Duration::from_secs(1));
    assert_eq!(
        saiu.iter().map(|(o, _)| *o).collect::<Vec<_>>(),
        [Ouvido::Barulho, Ouvido::Voz]
    );
}

/// O microfone do robô lê PCM cru da saída de um programa. Aqui o programa é o `cat` de um arquivo com a palma e a fala.
#[test]
fn microfone_do_robo_ouve_pelo_programa_e_volta_quando_ele_cai() {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    let pasta = tempfile::tempdir().unwrap();
    let cru = pasta.path().join("palma_e_fala.raw");
    let bytes: Vec<u8> = palma_e_fala()
        .iter()
        .flat_map(|a| ((a * 32767.0) as i16).to_le_bytes())
        .collect();
    std::fs::write(&cru, bytes).unwrap();

    let (ouvidos, ditos) = (
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Mutex::new(Vec::new())),
    );
    let (o, d) = (ouvidos.clone(), ditos.clone());
    let nivel = Arc::new(AtomicU32::new(0));
    let parar = Arc::new(AtomicBool::new(false));
    let saidas = microfone::Saidas {
        ouvido: Box::new(move |x| o.lock().unwrap().push(x)),
        nivel: nivel.clone(),
        dizer: Box::new(move |m: Option<String>| d.lock().unwrap().push(m)),
    };
    let (p, args) = (parar.clone(), vec![cru.to_str().unwrap().to_owned()]);
    let trava = Trava::nova(Duration::from_millis(800));
    let laco = std::thread::spawn(move || {
        microfone::rodar_comando("cat", &args, trava, saidas, Duration::from_millis(20), p)
    });
    // O `cat` termina no fim do arquivo: o microfone diz que parou e sobe de novo. Duas voltas bastam.
    let limite = Instant::now() + Duration::from_secs(10);
    while ditos.lock().unwrap().iter().filter(|m| m.is_some()).count() < 2 {
        assert!(
            Instant::now() < limite,
            "o microfone não voltou: {:?}",
            ditos.lock().unwrap()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    parar.store(true, Ordering::Relaxed);
    laco.join().unwrap();
    let ouvidos = ouvidos.lock().unwrap();
    assert_eq!(
        &ouvidos[..2],
        [Ouvido::Barulho, Ouvido::Voz],
        "a palma e a fala da primeira volta"
    );
    let ditos = ditos.lock().unwrap();
    assert_eq!(ditos[0], None, "ouvindo");
    assert!(
        ditos[1]
            .as_deref()
            .is_some_and(|m| m.contains("o microfone parou")),
        "{ditos:?}"
    );

    // O programa que não existe é dito, e o serviço não cai.
    let ditos = Arc::new(Mutex::new(Vec::new()));
    let d = ditos.clone();
    let parar = Arc::new(AtomicBool::new(false));
    let saidas = microfone::Saidas {
        ouvido: Box::new(|_| {}),
        nivel: Arc::new(AtomicU32::new(0)),
        dizer: Box::new(move |m: Option<String>| d.lock().unwrap().push(m)),
    };
    let p = parar.clone();
    let trava = Trava::nova(Duration::from_millis(800));
    let laco = std::thread::spawn(move || {
        microfone::rodar_comando(
            "programa-que-nao-existe",
            &[],
            trava,
            saidas,
            Duration::from_millis(5),
            p,
        )
    });
    std::thread::sleep(Duration::from_millis(60));
    parar.store(true, Ordering::Relaxed);
    laco.join().unwrap();
    assert!(
        ditos.lock().unwrap()[0]
            .as_deref()
            .is_some_and(|m| m.contains("não abriu"))
    );
    assert!((microfone::nivel(&[0.5, -0.5, 0.5, -0.5]) - 0.5).abs() < 1e-6);
}
