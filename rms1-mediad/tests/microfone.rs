//! O microfone e a trava: os critérios 5 a 7 do plano.

use std::time::{Duration, Instant};

use rms1_mediad::microfone::{self, BLOCO, Ouvido, Ouvinte, Voz};
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
    let trechos = Arc::new(Mutex::new(Vec::new()));
    let t = trechos.clone();
    let saidas = microfone::Saidas {
        ouvido: Box::new(move |x| o.lock().unwrap().push(x)),
        nivel: nivel.clone(),
        dizer: Box::new(move |m: Option<String>| d.lock().unwrap().push(m)),
        trecho: Box::new(move |x: Vec<i16>| t.lock().unwrap().push(x)),
    };
    let (p, args) = (parar.clone(), vec![cru.to_str().unwrap().to_owned()]);
    let ouvinte = Ouvinte::com_voz(Trava::nova(Duration::from_millis(800)), voz());
    let laco = std::thread::spawn(move || {
        microfone::rodar_comando("cat", &args, ouvinte, saidas, Duration::from_millis(20), p)
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
    // E a fala da primeira volta saiu como trecho, pelo mesmo caminho: é o que vai à estação.
    let falas = trechos.lock().unwrap();
    assert!(
        !falas.is_empty() && (27_000..=37_000).contains(&falas[0].len()),
        "{:?}",
        falas.iter().map(Vec::len).collect::<Vec<_>>()
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
        trecho: Box::new(|_| {}),
    };
    let p = parar.clone();
    let ouvinte = Ouvinte::novo(Trava::nova(Duration::from_millis(800)));
    let laco = std::thread::spawn(move || {
        microfone::rodar_comando(
            "programa-que-nao-existe",
            &[],
            ouvinte,
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

// ---- o trecho de voz que vai à estação (2026-11-17) ------------------------------------------------------------------------

/// Os números de `[midia.voz]`, os padrões.
fn voz() -> Voz {
    Voz {
        folga: Duration::from_millis(300),
        silencio: Duration::from_millis(700),
        teto: Duration::from_secs(10),
        minimo: Duration::from_millis(400),
        entra: 3.0,
        sai: 2.0,
    }
}

/// Passa as amostras pelo ouvinte, em blocos, com o relógio no ritmo delas, e devolve os trechos de voz que saíram.
fn trechos(ouvinte: &mut Ouvinte, amostras: &[f32], t0: Instant) -> Vec<Vec<i16>> {
    let mut saiu = Vec::new();
    for (i, bloco) in amostras.chunks(BLOCO).enumerate() {
        let quando = Duration::from_secs_f64((i * BLOCO) as f64 / HZ as f64);
        ouvinte.empurrar(bloco, t0 + quando);
        saiu.extend(ouvinte.trecho());
    }
    saiu
}

/// Quantas amostras do trecho são de som alto: as da fala de mentira, de amplitude 0,2.
fn altas(trecho: &[i16]) -> usize {
    trecho.iter().filter(|a| a.unsigned_abs() > 300).count()
}

#[test]
fn a_fala_sai_inteira_com_a_folga_de_antes_e_a_palma_nao_sai() {
    let mut ouvinte = Ouvinte::com_voz(Trava::nova(Duration::from_millis(800)), voz());
    let saiu = trechos(&mut ouvinte, &palma_e_fala(), Instant::now());
    assert_eq!(
        saiu.len(),
        1,
        "a palma, de 0,1 s, não é pedido; a fala, de 1 s, é: {:?}",
        saiu.iter().map(Vec::len).collect::<Vec<_>>()
    );
    let fala = &saiu[0];
    // A folga de antes (0,3 s), a fala (1 s) e o silêncio que a encerra (0,7 s): 2 s, com a folga de um quadro de 32 ms.
    assert!(
        (31_000..=33_500).contains(&fala.len()),
        "{} amostras",
        fala.len()
    );
    // A fala está toda lá (1 s a 16 kHz; o ruído de mentira passa de 300 em 95% das amostras, e o da sala nunca)...
    assert!(altas(fala) > 14_000, "{} amostras altas", altas(fala));
    // ...e o trecho começa antes dela: o primeiro décimo de segundo é a sala quieta.
    assert_eq!(altas(&fala[..1600]), 0, "o trecho começou no meio da fala");
    // O ouvinte que não guarda voz não entrega trecho nenhum.
    let mut sem = Ouvinte::novo(Trava::nova(Duration::from_millis(800)));
    assert!(trechos(&mut sem, &palma_e_fala(), Instant::now()).is_empty());
}

/// A sala e a fala medidas no S1 em 2026-11-18, com o ar-condicionado ligado: o silêncio em 0,0023 e a fala normal, a meio
/// metro, perto de 0,012, umas cinco vezes o silêncio. Com o corte em seis vezes, essa fala não saía.
#[test]
fn a_fala_normal_a_cinco_vezes_o_silencio_da_sala_sai_inteira() {
    let mut s = 11u32;
    // O ruído uniforme de amplitude `a` tem nível `a / raiz de 3`.
    let (sala, fala) = (0.0023 * 3f32.sqrt(), 0.012 * 3f32.sqrt());
    let mut a = ruido(sala, 12.0, &mut s);
    a.extend(ruido(fala, 1.5, &mut s));
    a.extend(ruido(sala, 1.5, &mut s));
    let mut ouvinte = Ouvinte::com_voz(Trava::nova(Duration::from_millis(800)), voz());
    let saiu = trechos(&mut ouvinte, &a, Instant::now());
    assert_eq!(saiu.len(), 1, "a fala de 1,5 s sai, e só ela: {:?}", saiu.iter().map(Vec::len).collect::<Vec<_>>());
    // A folga de antes (0,3 s), a fala (1,5 s) e o silêncio que a encerra (0,7 s): 2,5 s, com a folga de dois quadros.
    assert!((39_000..=41_500).contains(&saiu[0].len()), "{} amostras", saiu[0].len());
}

/// A frase com uma pausa de um segundo no meio: com o silêncio de 0,7 s saía em dois trechos, e o segundo, sem o nome de
/// chamada, era recusado pela estação. Com 1,2 s (o padrão desde 2026-11-18), sai num só.
#[test]
fn a_pausa_de_um_segundo_no_meio_da_frase_nao_a_corta_em_duas() {
    let mut s = 23u32;
    let mut a = ruido(0.001, 3.0, &mut s);
    a.extend(ruido(0.2, 0.8, &mut s));
    a.extend(ruido(0.001, 1.0, &mut s));
    a.extend(ruido(0.2, 0.8, &mut s));
    a.extend(ruido(0.001, 2.0, &mut s));
    let com = |silencio_ms: u64| {
        let cfg = Voz { silencio: Duration::from_millis(silencio_ms), ..voz() };
        trechos(&mut Ouvinte::com_voz(Trava::nova(Duration::from_millis(800)), cfg), &a, Instant::now()).len()
    };
    assert_eq!((com(700), com(1200)), (2, 1), "com 0,7 s a pausa corta a frase; com 1,2 s, não");
}

#[test]
fn a_frase_longa_sai_inteira_e_o_que_passa_do_teto_e_cortado_nele() {
    // Cinco segundos de fala: o vigia de som trata mais de 3 s como ruído contínuo e não avisa. O trecho sai assim mesmo.
    let mut s = 11u32;
    let mut audio = ruido(0.001, 2.0, &mut s);
    audio.extend(ruido(0.2, 5.0, &mut s));
    audio.extend(ruido(0.001, 1.5, &mut s));
    let mut ouvinte = Ouvinte::com_voz(Trava::nova(Duration::from_millis(800)), voz());
    let saiu = trechos(&mut ouvinte, &audio, Instant::now());
    assert_eq!(
        saiu.len(),
        1,
        "{:?}",
        saiu.iter().map(Vec::len).collect::<Vec<_>>()
    );
    assert!(
        altas(&saiu[0]) > 5 * 14_000,
        "{} amostras altas",
        altas(&saiu[0])
    );

    // Com o teto em 2 s, o som que não para sai uma vez, cortado no teto, e não volta a sair: o piso passa a contá-lo.
    let mut s = 13u32;
    let mut audio = ruido(0.001, 2.0, &mut s);
    audio.extend(ruido(0.2, 6.0, &mut s));
    let curto = Voz {
        teto: Duration::from_secs(2),
        ..voz()
    };
    let mut ouvinte = Ouvinte::com_voz(Trava::nova(Duration::from_millis(800)), curto);
    let saiu = trechos(&mut ouvinte, &audio, Instant::now());
    assert_eq!(
        saiu.len(),
        1,
        "{:?}",
        saiu.iter().map(Vec::len).collect::<Vec<_>>()
    );
    assert!(
        (31_400..=32_000).contains(&saiu[0].len()),
        "{} amostras, com o teto em 32.000",
        saiu[0].len()
    );
}

#[test]
fn a_propria_voz_nao_vira_trecho() {
    let trava = Trava::nova(Duration::from_millis(800));
    let mut ouvinte = Ouvinte::com_voz(trava.clone(), voz());
    let t0 = Instant::now();
    // O alto-falante toca durante o áudio inteiro: a fala que o microfone ouve é a do robô.
    trava.comecou();
    assert!(trechos(&mut ouvinte, &palma_e_fala(), t0).is_empty());
    // O som começa no meio de uma fala de alguém: o trecho em curso é largado, e não sai pela metade.
    let fim = t0 + Duration::from_secs(8);
    trava.terminou(fim);
    let depois = fim + Duration::from_secs(1);
    let audio = palma_e_fala();
    let meio = (5.6 * HZ as f32) as usize;
    assert!(trechos(&mut ouvinte, &audio[..meio], depois).is_empty());
    trava.comecou();
    let resto = depois + Duration::from_secs_f64(meio as f64 / HZ as f64);
    assert!(trechos(&mut ouvinte, &audio[meio..], resto).is_empty());
    // Passada a janela depois do som, a fala seguinte sai.
    let fim = resto + Duration::from_secs(2);
    trava.terminou(fim);
    let saiu = trechos(&mut ouvinte, &palma_e_fala(), fim + Duration::from_secs(1));
    assert_eq!(
        saiu.len(),
        1,
        "{:?}",
        saiu.iter().map(Vec::len).collect::<Vec<_>>()
    );
}
