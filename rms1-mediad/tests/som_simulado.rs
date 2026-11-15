//! O som do robô simulado: o alto-falante manda à página, pelo serviço local, o que toca, e o microfone ouve o que ela manda.
//! O serviço local, aqui, é uma porta de mentira.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rms1_mediad::alto_falante::{Simulador, Tocador, duracao_wav, recado};
use rms1_mediad::microfone::{self, BLOCO, OUVE, Saidas};
use rms1_mediad::trava::Trava;

/// Um banco com um som de `ms` milissegundos em cada pasta.
fn banco(ms: u32) -> tempfile::TempDir {
    let pasta = tempfile::tempdir().unwrap();
    for tag in ["chirp", "greet"] {
        std::fs::create_dir_all(pasta.path().join(tag)).unwrap();
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(pasta.path().join(tag).join("a.wav"), spec).unwrap();
        for i in 0..(44_100 * ms / 1000) {
            w.write_sample((i % 100) as i16).unwrap();
            w.write_sample(0i16).unwrap();
        }
        w.finalize().unwrap();
    }
    pasta
}

/// Lê um recado inteiro: o tipo e o conteúdo.
fn recado_de(c: &mut TcpStream) -> (u8, Vec<u8>) {
    let mut cabeca = [0u8; 5];
    c.read_exact(&mut cabeca).unwrap();
    let mut corpo = vec![0u8; u32::from_le_bytes(cabeca[1..].try_into().unwrap()) as usize];
    c.read_exact(&mut corpo).unwrap();
    (cabeca[0], corpo)
}

fn aceitar(porta: &TcpListener, papel: u8) -> TcpStream {
    let (mut c, _) = porta.accept().unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut b = [0u8; 1];
    c.read_exact(&mut b).unwrap();
    assert_eq!(b[0], papel, "quem ligou disse o papel errado");
    c
}

#[test]
fn o_alto_falante_do_simulador_manda_o_arquivo_e_dura_o_que_ele_dura() {
    let pasta = banco(300);
    let porta = TcpListener::bind("127.0.0.1:0").unwrap();
    let endereco = porta.local_addr().unwrap().to_string();
    let trava = Trava::nova(Duration::from_millis(200));
    let mut tocador = Tocador::novo(
        pasta.path().to_owned(),
        Box::new(Simulador { endereco }),
        trava.clone(),
    );
    let t0 = Instant::now();
    tocador.tocar("chirp", t0);
    let mut c = aceitar(&porta, recado::FALA);
    let (tipo, corpo) = recado_de(&mut c);
    let arquivo = std::fs::read(pasta.path().join("chirp").join("a.wav")).unwrap();
    assert_eq!(tipo, recado::ARQUIVO);
    assert_eq!(corpo, arquivo, "a página tem de receber o arquivo inteiro");
    let dura = duracao_wav(&arquivo).unwrap();
    assert!(
        (dura.as_secs_f64() - 0.3).abs() < 0.01,
        "a duração sai do cabeçalho: {dura:?}"
    );
    // Enquanto o som dura, o tocador está tocando e a trava vale.
    tocador.tique(Instant::now());
    assert!(tocador.tocando(), "o som acabou antes da hora");
    assert!(trava.vale(Instant::now()));
    std::thread::sleep(Duration::from_millis(350));
    tocador.tique(Instant::now());
    assert!(
        !tocador.tocando(),
        "o som não terminou depois da duração dele"
    );
}

#[test]
fn um_som_novo_corta_o_anterior_na_pagina() {
    let pasta = banco(2000);
    let porta = TcpListener::bind("127.0.0.1:0").unwrap();
    let endereco = porta.local_addr().unwrap().to_string();
    let mut tocador = Tocador::novo(
        pasta.path().to_owned(),
        Box::new(Simulador { endereco }),
        Trava::nova(Duration::from_millis(200)),
    );
    tocador.tocar("chirp", Instant::now());
    let mut primeiro = aceitar(&porta, recado::FALA);
    assert_eq!(recado_de(&mut primeiro).0, recado::ARQUIVO);
    tocador.tocar("greet", Instant::now());
    assert_eq!(
        recado_de(&mut primeiro),
        (recado::CORTE, Vec::new()),
        "o som anterior não foi cortado na página"
    );
    let mut segundo = aceitar(&porta, recado::FALA);
    assert_eq!(recado_de(&mut segundo).0, recado::ARQUIVO);
}

#[test]
fn sem_o_servico_local_o_som_e_recusado_e_nada_fica_tocando() {
    let pasta = banco(300);
    // Uma porta que ninguém ouve.
    let livre = TcpListener::bind("127.0.0.1:0").unwrap();
    let endereco = livre.local_addr().unwrap().to_string();
    drop(livre);
    let trava = Trava::nova(Duration::from_millis(200));
    let mut tocador = Tocador::novo(
        pasta.path().to_owned(),
        Box::new(Simulador { endereco }),
        trava.clone(),
    );
    tocador.tocar("chirp", Instant::now());
    assert!(!tocador.tocando());
    assert!(
        !trava.vale(Instant::now()),
        "a trava prendeu sem som nenhum"
    );
}

#[test]
fn o_microfone_do_simulador_ouve_o_que_a_pagina_manda() {
    let porta = TcpListener::bind("127.0.0.1:0").unwrap();
    let endereco = porta.local_addr().unwrap().to_string();
    let nivel = Arc::new(AtomicU32::new(0));
    let dito: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
    let parar = Arc::new(AtomicBool::new(false));
    let (d, n, p) = (dito.clone(), nivel.clone(), parar.clone());
    let fio = std::thread::spawn(move || {
        let saidas = Saidas {
            ouvido: Box::new(|_| {}),
            nivel: n,
            dizer: Box::new(move |m| d.lock().unwrap().push(m)),
        };
        microfone::rodar_simulador(
            &endereco,
            Trava::nova(Duration::from_millis(200)),
            saidas,
            Duration::from_millis(100),
            p,
        );
    });
    let mut c = aceitar(&porta, OUVE);
    // Um bloco e meio de onda quadrada de amplitude 0,5: só o bloco inteiro é ouvido, e o resto espera o que falta.
    let amostra = |i: usize| {
        if i.is_multiple_of(2) {
            16384i16
        } else {
            -16384i16
        }
    };
    let bytes: Vec<u8> = (0..BLOCO * 3 / 2)
        .flat_map(|i| amostra(i).to_le_bytes())
        .collect();
    c.write_all(&bytes).unwrap();
    let ate = Instant::now() + Duration::from_secs(3);
    let ler = || f32::from_bits(nivel.load(Ordering::Relaxed));
    while ler() < 0.4 && Instant::now() < ate {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!((ler() - 0.5).abs() < 0.01, "o nível do bloco: {}", ler());
    // A página calou: o nível volta a zero, e isso não é defeito do microfone.
    let ate = Instant::now() + Duration::from_secs(3);
    while ler() > 0.0 && Instant::now() < ate {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(ler(), 0.0, "com a página calada o nível não voltou a zero");
    assert!(
        dito.lock().unwrap().iter().all(Option::is_none),
        "o microfone disse um defeito que não há: {:?}",
        dito.lock().unwrap()
    );
    // A outra metade do bloco chega depois do silêncio: o bloco se completa, sem perder o que já tinha vindo.
    let resto: Vec<u8> = (0..BLOCO / 2)
        .flat_map(|i| amostra(i).to_le_bytes())
        .collect();
    c.write_all(&resto).unwrap();
    let ate = Instant::now() + Duration::from_secs(3);
    while ler() < 0.4 && Instant::now() < ate {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        (ler() - 0.5).abs() < 0.01,
        "o bloco partido pelo silêncio se perdeu: {}",
        ler()
    );
    parar.store(true, Ordering::Relaxed);
    drop(c);
    fio.join().unwrap();
}
