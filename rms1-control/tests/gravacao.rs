//! A comparação que prova que simulador e robô receberam os mesmos quadros.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rms1_control::corpo::{Corpo, CorpoFalso};
use rms1_control::gravacao::{Gravando, diferencas};
use rms1_control::{Intencoes, Laco, Saude};
use rms1d_params::Params;

fn gravar(caminho: &std::path::Path, ciclos_por_pedido: usize, vx: f64) {
    let corpo: Box<dyn Corpo> = Box::new(CorpoFalso::default());
    let gravando = Gravando::new(corpo, caminho).unwrap();
    let intencoes = Arc::new(Intencoes::default());
    let mut laco = Laco::new(
        gravando,
        &Params::default(),
        intencoes.clone(),
        Arc::new(Saude::default()),
    );
    intencoes.pedir_armar(true);
    let mut t = Instant::now();
    for _ in 0..ciclos_por_pedido {
        intencoes.pedir_twist([vx, 0.0, 0.0]);
        t = Instant::now().max(t + Duration::from_millis(1));
        laco.ciclo(t, false);
    }
    intencoes.pedir_torre(0.2, 0.1);
    laco.ciclo(t, false);
    intencoes.pedir_armar(false);
    laco.ciclo(t, false);
}

/// Duração diferente, mesma sequência de conteúdos: igual. É o caso de simulador e robô.
#[test]
fn o_mesmo_roteiro_com_tempos_diferentes_e_igual() {
    let d = tempfile_dir();
    let (a, b) = (d.join("a"), d.join("b"));
    gravar(&a, 3, 1.0);
    gravar(&b, 30, 1.0);
    assert_eq!(diferencas(&a, &b).unwrap(), Vec::<String>::new());
}

#[test]
fn um_comando_diferente_aparece_na_comparacao() {
    let d = tempfile_dir();
    let (a, b) = (d.join("a"), d.join("b"));
    gravar(&a, 3, 1.0);
    gravar(&b, 3, 0.5);
    let dif = diferencas(&a, &b).unwrap();
    assert_eq!(dif.len(), 1, "{dif:?}");
    assert!(dif[0].contains("0x3f/0x20"), "{dif:?}");
}

fn tempfile_dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "rms1-gravacao-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}
