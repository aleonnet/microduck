//! O programa `rms1-mundo-reproduz`, rodado de verdade sobre uma sessão gravada: as janelas conferidas contra a distância de
//! cada uma, o veredito numa linha só, e o exame de cada grade, que é o que a calibração mede.

mod comum;

use std::process::Command;

use comum::{amostra, caixa_em, parede_em};
use rms1_mundo::{Ajustes, Mundo};

const K: f64 = 0.1;

/// Uma sessão com a caixa a 1 m do centro do corpo por um segundo, e depois a 1,5 m por outro: grava e devolve o arquivo.
fn gravar(pasta: &std::path::Path) -> std::path::PathBuf {
    let arquivo = pasta.join("sessao.mundo");
    let ajustes = Ajustes {
        gravar: arquivo.to_str().unwrap().to_owned(),
        ..comum::ajustes()
    };
    let olho = comum::olho_do_simulador();
    let mut m = Mundo::novo(Some(olho), comum::corpo(), ajustes).unwrap();
    for quadro in 0..30 {
        let t = 5.0 + f64::from(quadro) / 15.0;
        let face = if quadro < 15 { 1.0 } else { 1.5 };
        let a = amostra(t, 0.0, 0.0, [0.0; 3]);
        m.amostra(a);
        let cena = [caixa_em(face, 0.0), parede_em(4.0)];
        m.grade(&comum::ver(&olho, &a, &cena, K), "scdepthv3", t)
            .unwrap();
    }
    arquivo
}

fn rodar(arquivo: &std::path::Path, opcoes: &[&str]) -> (Option<i32>, String) {
    let saida = Command::new(env!("CARGO_BIN_EXE_rms1-mundo-reproduz"))
        .arg(arquivo)
        .args(opcoes)
        .output()
        .unwrap();
    (
        saida.status.code(),
        String::from_utf8_lossy(&saida.stdout).into_owned()
            + &String::from_utf8_lossy(&saida.stderr),
    )
}

#[test]
fn as_janelas_sao_conferidas_cada_uma_contra_a_distancia_dela_e_o_veredito_e_um_so() {
    let pasta = tempfile::tempdir().unwrap();
    let arquivo = gravar(pasta.path());
    // A frente do chassi fica a 0,16 m do centro: a caixa a 1 m está a 0,84 m dela, e a 1,5 m, a 1,34 m. A segunda janela
    // começa depois de a caixa de antes ter sido vista livre nos quadros seguidos que a apagam.
    let (codigo, texto) = rodar(
        &arquivo,
        &[
            "--calado", "--janela", "5.2", "5.9", "0.84", "--janela", "6.4", "7.0", "1.34",
        ],
    );
    assert_eq!(codigo, Some(0), "{texto}");
    assert_eq!(texto.matches("dentro da regra").count(), 2, "{texto}");
    assert_eq!(texto.lines().last(), Some("REPRODUCAO CONFERE"), "{texto}");
    assert_eq!(texto.matches("REPRODUCAO").count(), 1, "{texto}");

    // Uma das janelas com a distância errada: o veredito é um só, e reprova.
    let (codigo, texto) = rodar(
        &arquivo,
        &[
            "--calado", "--janela", "5.2", "5.9", "0.84", "--janela", "6.4", "7.0", "0.84",
        ],
    );
    assert_eq!(codigo, Some(1), "{texto}");
    assert!(texto.contains("FORA da regra"), "{texto}");
    assert_eq!(
        texto.lines().last(),
        Some("REPRODUCAO NAO CONFERE"),
        "{texto}"
    );

    // A janela sem nenhum mundo com medida reprova, e não passa calada.
    let (codigo, texto) = rodar(&arquivo, &["--calado", "--janela", "20", "21", "0.84"]);
    assert_eq!(codigo, Some(1), "{texto}");
    assert!(
        texto.contains("nenhum mundo com ocupado no corredor"),
        "{texto}"
    );

    // A janela escrita ao contrário é recusada pelo motivo.
    let (codigo, texto) = rodar(&arquivo, &["--janela", "6", "5", "0.84"]);
    assert_eq!(codigo, Some(2), "{texto}");
    assert!(texto.contains("o fim depois do começo"), "{texto}");
}

#[test]
fn com_exame_cada_linha_leva_a_escala_o_exame_e_as_ocupadas_do_quadro() {
    let pasta = tempfile::tempdir().unwrap();
    let arquivo = gravar(pasta.path());
    let (codigo, texto) = rodar(&arquivo, &["--exame"]);
    assert_eq!(codigo, Some(0), "{texto}");
    let linhas: Vec<serde_json::Value> = texto
        .lines()
        .filter(|l| l.starts_with('{'))
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(linhas.len(), 30);
    let primeira = &linhas[0];
    assert_eq!(primeira["exam"]["origin"], "measured", "{primeira}");
    let escala = primeira["exam"]["scale"].as_f64().unwrap();
    assert!((escala / K - 1.0).abs() < 0.02, "a escala saiu {escala}");
    assert!(primeira["exam"]["frame"]["refused"].is_null(), "{primeira}");
    assert!(
        primeira["exam"]["frame"]["deviation"].as_f64().unwrap() < 0.05,
        "{primeira}"
    );
    let ocupadas = primeira["fresh"].as_array().unwrap();
    assert!(
        ocupadas
            .iter()
            .any(|c| (c[0].as_f64().unwrap() - 1.0).abs() < 0.08),
        "as ocupadas do primeiro quadro não estão na face da caixa a 1 m: {primeira}"
    );
    // Sem `--exame`, a linha é a de sempre.
    let (_, texto) = rodar(&arquivo, &[]);
    assert!(
        !texto.contains("\"exam\"") && !texto.contains("\"fresh\""),
        "{texto}"
    );
}
