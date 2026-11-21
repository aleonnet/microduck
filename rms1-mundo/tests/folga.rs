//! O mundo pedido para agora: a folga por direção com a idade, o não visto onde não houve observação, o obstáculo na forma do
//! cérebro do pato, mais perto depois de o robô andar; e o estado da percepção, em que "falhou" não é "não vi nada".

mod comum;

use comum::{amostra, caixa_em, entrega, parede_em, profundidades};
use rms1_mundo::{Amostra, Corpo, Estado, Mundo, Olho, Percepcao, Raios, Setor};

const K: f64 = 0.1;

fn olhar(m: &mut Mundo, olho: &Olho, a: Amostra, caixas: &[comum::Caixa]) {
    m.amostra(a);
    m.grade(&comum::ver(olho, &a, caixas, K), "scdepthv3", a.t)
        .unwrap();
}

fn setor(folga: &[Setor], rumo_grau: f64) -> Setor {
    *folga
        .iter()
        .find(|s| (s.rumo_grau - rumo_grau).abs() < 1e-6)
        .unwrap_or_else(|| panic!("não há setor a {rumo_grau}°"))
}

#[test]
fn a_folga_por_direcao_e_o_obstaculo_a_frente() {
    let olho = comum::olho_do_simulador();
    let corpo = comum::corpo();
    let mut m = comum::mundo(olho);
    assert_eq!(
        m.agora(0.5).obstaculo,
        None,
        "antes da primeira grade, o corredor nunca foi visto"
    );
    olhar(
        &mut m,
        &olho,
        amostra(1.0, 0.0, 0.0, [0.0; 3]),
        &[caixa_em(1.0, 0.0), parede_em(4.0)],
    );
    let agora = m.agora(1.0);
    assert_eq!(agora.percepcao, Percepcao::Ok);
    assert_eq!(agora.folga.len(), comum::ajustes().setores as usize);
    // À frente, a caixa: a 1 m do centro do corpo, dentro de uma célula e meia.
    let frente = setor(&agora.folga, 0.0);
    assert_eq!(frente.estado, Estado::Ocupada);
    assert!(
        (frente.livre_m - 1.0).abs() <= 0.06,
        "a caixa a 1 m saiu a {} m no setor da frente",
        frente.livre_m
    );
    assert!(frente.idade_s < 1e-9);
    // A 40° para a esquerda, dentro da vista e fora da caixa: o chão livre até o alcance.
    let esquerda = setor(&agora.folga, 40.0);
    assert_eq!(esquerda.estado, Estado::Livre);
    assert!(
        esquerda.livre_m > 2.5 && esquerda.livre_m <= 3.0,
        "livre até {} m a 40°",
        esquerda.livre_m
    );
    // Atrás e de lado, fora da vista: não visto, que não é livre.
    for rumo in [180.0, 90.0, -90.0, -120.0] {
        assert_eq!(
            setor(&agora.folga, rumo).estado,
            Estado::NaoVista,
            "a {rumo}° nada foi visto"
        );
    }
    // O obstáculo, na forma do cérebro do pato: da frente do chassi à caixa.
    let o = agora.obstaculo.expect("o corredor foi visto");
    assert!(
        (o.ahead_m - (1.0 - corpo.frente_m)).abs() <= 0.06,
        "da frente do chassi à caixa são {} m, e saiu {}",
        1.0 - corpo.frente_m,
        o.ahead_m
    );
    assert!(o.age_s < 1e-9);

    // O robô anda 30 cm para a frente, sem grade nova: o lembrado chega mais perto.
    m.amostra(amostra(1.1, 0.0, 0.0, [0.3, 0.0, 0.0]));
    let depois = m.agora(1.1);
    let o2 = depois.obstaculo.unwrap();
    assert!(
        (o.ahead_m - o2.ahead_m - 0.3).abs() <= 0.02,
        "depois de andar 30 cm, a folga foi de {} para {}",
        o.ahead_m,
        o2.ahead_m
    );
    assert!(
        (o2.age_s - 0.1).abs() < 1e-9,
        "a idade do que foi visto: {}",
        o2.age_s
    );
    assert!((setor(&depois.folga, 0.0).livre_m - 0.7).abs() <= 0.06);
    // O chassi gira 90° à esquerda: a caixa passa a ficar à direita do corpo, e o corredor à frente não tem nada lembrado.
    m.amostra(amostra(1.2, 0.0, 0.0, [0.3, 0.0, 90.0]));
    let girado = m.agora(1.2);
    assert_eq!(
        setor(&girado.folga, -90.0).estado,
        Estado::Ocupada,
        "com o chassi girado, a caixa fica à direita do corpo"
    );
    assert!(
        girado.obstaculo.is_none_or(|o| o.ahead_m.is_infinite()),
        "o corredor novo não tem ocupado: {:?}",
        girado.obstaculo
    );
}

#[test]
fn o_lado_livre_e_o_de_longe_do_obstaculo() {
    let olho = comum::olho_do_simulador();
    for (y, lado) in [(-0.5, 1.0), (0.5, -1.0)] {
        let mut m = comum::mundo(olho);
        olhar(
            &mut m,
            &olho,
            amostra(1.0, 0.0, 0.0, [0.0; 3]),
            &[caixa_em(0.8, y), parede_em(4.0)],
        );
        let o = m.agora(1.0).obstaculo.expect("o corredor foi visto");
        assert_eq!(
            o.away, lado,
            "com a caixa em y = {y}, o lado livre é o outro"
        );
        assert!(
            o.ahead_m.is_infinite(),
            "a caixa fica fora do corredor: {}",
            o.ahead_m
        );
    }
    // O corredor tem a largura do corpo: a caixa que encosta nele barra, a que fica um palmo fora não.
    let corpo = comum::corpo();
    for (y, barra) in [
        (corpo.largura_m / 2.0 + 0.15 - 0.03, true),
        (corpo.largura_m / 2.0 + 0.15 + 0.2, false),
    ] {
        let mut m = Mundo::novo(Some(olho), Corpo { ..corpo }, comum::ajustes()).unwrap();
        olhar(
            &mut m,
            &olho,
            amostra(1.0, 0.0, 0.0, [0.0; 3]),
            &[caixa_em(0.8, y), parede_em(4.0)],
        );
        let o = m.agora(1.0).obstaculo.unwrap();
        assert_eq!(
            o.ahead_m.is_finite(),
            barra,
            "a caixa com o meio em y = {y}: {}",
            o.ahead_m
        );
    }
}

#[test]
fn o_estado_da_percepcao_diz_por_que_nao_ha_medida() {
    let olho = comum::olho_do_simulador();
    let ajustes = comum::ajustes();
    let cena = [caixa_em(1.0, 0.0), parede_em(4.0)];
    let a = amostra(1.0, 0.0, 0.0, [0.0; 3]);
    let carga = comum::ver(&olho, &a, &cena, K);

    // Sem a lente na tabela: o mundo só diz que ela falta.
    let mut m = Mundo::novo(None, comum::corpo(), comum::ajustes()).unwrap();
    m.amostra(a);
    assert_eq!(
        m.grade(&carga, "scdepthv3", 1.0).unwrap().percepcao,
        Percepcao::SemCamera
    );
    let agora = m.agora(1.0);
    assert_eq!((agora.percepcao.nome(), agora.lente), ("no_camera", None));
    assert!(m.celulas(1.0).is_empty());

    // Sem a torre e a pose: nenhuma amostra, ou a mais perto longe demais do instante do quadro.
    let mut m = comum::mundo(olho);
    assert_eq!(
        m.grade(&carga, "scdepthv3", 1.0).unwrap().percepcao,
        Percepcao::SemPose
    );
    assert_eq!(m.agora(1.0).percepcao.nome(), "no_pose");
    m.amostra(amostra(
        1.0 - ajustes.torre_vale_ms as f64 / 1000.0 - 0.1,
        0.0,
        0.0,
        [0.0; 3],
    ));
    assert_eq!(
        m.grade(&carga, "scdepthv3", 1.0).unwrap().percepcao,
        Percepcao::SemPose
    );
    assert!(
        m.celulas(1.0).is_empty(),
        "sem a pose do instante, nada entra no mapa"
    );

    // O leitor que não dá metros.
    let mut m = comum::mundo(olho);
    m.amostra(a);
    assert_eq!(
        m.grade(&carga, "fast_depth", 1.0).unwrap().percepcao,
        Percepcao::SemMetros
    );
    assert_eq!(m.agora(1.0).percepcao.nome(), "no_metres");
    // E a grade cuja espécie não é a do `scdepthv3`, venha com o nome que vier.
    let mut de_outra = carga.clone();
    de_outra[5] = 1;
    assert_eq!(
        m.grade(&de_outra, "scdepthv3", 1.0).unwrap().percepcao,
        Percepcao::SemMetros
    );

    // A grade que não se lê é erro de quem a entregou, e o mundo fica como estava.
    let mut m = comum::mundo(olho);
    m.amostra(a);
    m.grade(&carga, "scdepthv3", 1.0).unwrap();
    let antes = m.agora(1.0);
    for ruim in [&carga[..10], &carga[..carga.len() - 1], &[][..]] {
        let erro = m.grade(ruim, "scdepthv3", 1.05).unwrap_err();
        assert!(erro.contains("grade"), "{erro}");
    }
    assert_eq!(m.agora(1.0), antes);

    // A grade para de chegar: passado o prazo, a percepção está velha; e a idade do quadro é dita.
    m.amostra(amostra(1.0 + ajustes.velho_s + 0.1, 0.0, 0.0, [0.0; 3]));
    let velho = m.agora(1.0 + ajustes.velho_s + 0.1);
    assert_eq!((velho.percepcao.nome(), velho.escala), ("stale", None));
    assert!((velho.idade_do_quadro_ms.unwrap() - (ajustes.velho_s + 0.1) * 1000.0).abs() < 1e-6);
    // O lembrado segue no mapa, com a idade: quem lê sabe que é velho, e não toma por livre.
    assert_eq!(setor(&velho.folga, 0.0).estado, Estado::Ocupada);
    assert!(setor(&velho.folga, 0.0).idade_s > ajustes.velho_s);

    // A estação diz que a função falhou: o estado é a falha, com o motivo, até a grade seguinte.
    let mut m = comum::mundo(olho);
    m.amostra(a);
    m.grade(&carga, "scdepthv3", 1.0).unwrap();
    m.amostra(amostra(1.07, 0.0, 0.0, [0.0; 3]));
    m.falha(1.07, "fila cheia no modelo");
    let falhou = m.agora(1.07);
    assert_eq!(
        (falhou.percepcao.nome(), falhou.motivo.as_deref()),
        ("failed", Some("fila cheia no modelo"))
    );
    m.amostra(amostra(1.14, 0.0, 0.0, [0.0; 3]));
    m.grade(&carga, "scdepthv3", 1.14).unwrap();
    assert_eq!(
        m.agora(1.14).percepcao,
        Percepcao::Ok,
        "a grade seguinte tira a falha"
    );
}

/// A linha de `s1.world` e a de `s1.world.grid`, como vão a quem assina.
#[test]
fn as_duas_linhas_levam_o_mundo_a_quem_assina() {
    let olho = comum::olho_do_simulador();
    let mut m = comum::mundo(olho);
    olhar(
        &mut m,
        &olho,
        amostra(1.0, 0.0, 0.0, [0.2, -0.1, 30.0]),
        &[caixa_em(1.2, 0.5), parede_em(6.0)],
    );
    let linha = m.agora(1.04).linha();
    assert_eq!(linha["frame"], "body");
    assert_eq!(linha["frame_age_ms"], 40.0);
    // A lente do simulador é a do robô: 54 graus na vertical, e 84,34 na horizontal.
    assert_eq!(
        (&linha["lens"]["hfov_deg"], &linha["lens"]["vfov_deg"]),
        (&serde_json::json!(84.34), &serde_json::json!(54.0))
    );
    assert_eq!(
        linha["lens"]["calibrated"], true,
        "a lente do simulador é exata por construção"
    );
    assert_eq!(
        linha["perception"],
        serde_json::json!({"state": "ok", "reason": null, "scale": "measured"})
    );
    let folga = linha["clearance"].as_array().unwrap();
    assert_eq!(folga.len(), 36);
    assert_eq!(folga[0][0], 0.0);
    assert_eq!(folga[1][0], 10.0);
    assert_eq!(folga[35][0], -10.0);
    assert!(folga.iter().all(|s| {
        s.as_array()
            .is_some_and(|s| s.len() == 4 && s[2].as_u64().is_some_and(|e| e <= 2))
    }));
    // O que não foi visto vai com a idade nula; o corredor livre, com `ahead_m` nulo.
    let atras = &folga[18];
    assert_eq!(
        (atras[0].as_f64(), atras[2].as_u64(), atras[3].is_null()),
        (Some(180.0), Some(0), true)
    );
    assert!(linha["obstacle"]["ahead_m"].is_null() || linha["obstacle"]["ahead_m"].is_number());
    assert!(linha["obstacle"]["away"] == 1.0 || linha["obstacle"]["away"] == -1.0);

    let grade = m.linha_da_grade(1.04);
    assert_eq!(grade["frame"], "odom");
    assert_eq!(grade["cell_m"], 0.04);
    assert_eq!(grade["pose"], serde_json::json!([0.2, -0.1, 30.0]));
    let celulas = grade["cells"].as_array().unwrap();
    assert!(celulas.len() > 100, "{}", celulas.len());
    assert!(celulas.iter().any(|c| c[2] == 2) && celulas.iter().any(|c| c[2] == 1));
    assert!(celulas.iter().all(|c| {
        c.as_array()
            .is_some_and(|c| c.len() == 4 && c[0].is_i64() && c[1].is_i64() && c[3] == 0.04)
    }));
    // Cada célula ocupada da linha é ocupada no mundo, no canto que a linha diz.
    for c in celulas.iter().filter(|c| c[2] == 2) {
        let meio = [
            (c[0].as_f64().unwrap() + 0.5) * 0.04,
            (c[1].as_f64().unwrap() + 0.5) * 0.04,
        ];
        assert_eq!(m.estado_em(meio, 1.04), Estado::Ocupada);
    }
}

/// A câmera do robô, com a lente provisória de fábrica: o mundo aparece do mesmo jeito (o chão, a caixa e os metros), e a
/// linha diz que a lente não está calibrada. Estar calibrado diz se a medida está certa, e não se ela aparece.
#[test]
fn com_a_lente_provisoria_o_mundo_aparece_e_diz_que_nao_esta_calibrado() {
    let olho = comum::olho_do_robo();
    assert!(olho.lente.provisoria);
    let mut m = comum::mundo(olho);
    for quadro in 0..3 {
        olhar(
            &mut m,
            &olho,
            amostra(1.0 + f64::from(quadro) / 15.0, 0.0, 0.0, [0.0; 3]),
            &[caixa_em(1.0, 0.0), parede_em(4.0)],
        );
    }
    let agora = m.agora(1.2);
    let linha = agora.linha();
    assert_eq!(
        linha["perception"],
        serde_json::json!({"state": "ok", "reason": null, "scale": "measured"}),
        "{linha}"
    );
    assert_eq!(linha["lens"]["calibrated"], false, "{linha}");
    let a_frente = agora.obstaculo.expect("o corredor foi visto").ahead_m;
    assert!(
        (a_frente - 0.84).abs() <= 0.06,
        "a caixa a 0,84 m da frente do chassi saiu a {a_frente} m"
    );
}

/// O cano do lançador na frente da lente do robô. Sem tirá-lo da conta, ele vira ocupado dentro do próprio corpo do robô;
/// tirado, o que o mundo diz é a cena: a escala do modelo, o chão livre e a caixa onde ela está.
#[test]
fn o_cano_do_lancador_nao_vira_obstaculo() {
    let olho = comum::olho_do_robo();
    let cena = [caixa_em(1.0, 0.0), parede_em(4.0)];
    // As células ocupadas a menos de 30 cm do centro do corpo: ali não há nada na cena.
    let coladas = |m: &Mundo, t: f64| {
        m.celulas(t)
            .into_iter()
            .filter(|(c, estado, _)| {
                let lado = comum::ajustes().celula_m;
                let (x, y) = ((f64::from(c[0]) + 0.5) * lado, (f64::from(c[1]) + 0.5) * lado);
                *estado == Estado::Ocupada && x.hypot(y) < 0.3
            })
            .count()
    };
    let ver = |m: &mut Mundo| {
        let mut ultima = None;
        for quadro in 0..3 {
            let a = amostra(1.0 + f64::from(quadro) / 15.0, 0.0, 0.0, [0.0; 3]);
            let mut prof = profundidades(&olho, &a, &cena);
            comum::por_o_cano(&olho, &mut prof);
            m.amostra(a);
            ultima = Some(m.grade(&entrega(&prof, K, None), "scdepthv3", a.t).unwrap());
        }
        ultima.unwrap()
    };

    // A câmera que não sabe do cano: ele entra no mapa, colado no robô.
    let mut cega = olho;
    cega.cano = None;
    let mut m = comum::mundo(cega);
    ver(&mut m);
    assert!(
        coladas(&m, 1.2) > 0,
        "sem tirar o cano da conta, ele tinha de aparecer como ocupado colado no robô: o teste não prova nada"
    );

    // A câmera do robô: o cano fica fora da escala e da ocupação.
    let mut m = comum::mundo(olho);
    let tratada = ver(&mut m);
    let escala = tratada.do_quadro.expect("o chão dá a escala do quadro");
    // A faixa de baixo é contada sem os pontos que olham para o cano, e nenhum deles entra na conta da escala.
    let raios = Raios::da_grade(
        &olho,
        comum::LARGURA,
        comum::ALTURA,
        comum::ajustes().corpo_folga_m,
    );
    let faixa = 36 * comum::LARGURA;
    let tapados_na_faixa = (comum::ALTURA * comum::LARGURA - faixa..comum::ALTURA * comum::LARGURA)
        .filter(|&k| raios.tapado(k))
        .count();
    let exame = tratada.exame.expect("o exame da faixa de baixo");
    assert!(tapados_na_faixa > 1000, "{tapados_na_faixa}");
    assert_eq!(
        exame.total,
        faixa - tapados_na_faixa,
        "a faixa de baixo tem de ser contada sem os pontos do cano"
    );
    assert!(
        exame.pontos <= exame.total,
        "pontos do cano entraram na conta da escala: {exame:?}"
    );
    assert!(
        (escala.valor / K - 1.0).abs() <= 0.02,
        "com o cano no quadro, a escala saiu {} e a do modelo é {K}",
        escala.valor
    );
    assert_eq!(
        coladas(&m, 1.2),
        0,
        "o cano do lançador virou ocupado colado no robô"
    );
    let a_frente = m.agora(1.2).obstaculo.expect("o corredor foi visto").ahead_m;
    assert!(
        (a_frente - 0.84).abs() <= 0.06,
        "com o cano no quadro, a caixa a 0,84 m da frente do chassi saiu a {a_frente} m"
    );
}
