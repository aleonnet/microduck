//! O formato que o S1 e a estação Hailo falam: o anúncio e o cabeçalho das mensagens. O critério 18 do plano e a parte de
//! endereço do 17. O pacote e a mensagem de `tests/fixtures/estacao/` foram gravados pelo lado Python
//! (`tools/hailo/estacao_anuncio.py gravar`, no laboratório), que confere, no teste dele, que são os que ele grava.

use std::net::{IpAddr, Ipv4Addr};

use rms1_mediad::estacao::{self, Cabecalho, Tipo};

const ANUNCIO: &[u8] = include_bytes!("fixtures/estacao/anuncio.json");
const MENSAGEM: &[u8] = include_bytes!("fixtures/estacao/mensagem.bin");

fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(a, b, c, d))
}

#[test]
fn le_o_formato_gravado_da_estacao() {
    let a = estacao::ler_anuncio(ANUNCIO, ip(192, 168, 1, 102))
        .expect("o anúncio gravado pelo lado Python");
    assert_eq!((a.quadros, a.painel), (47002, 8080));
    assert_eq!(a.entradas.len(), 1);
    let e = &a.entradas[0];
    assert_eq!(
        (e.n, e.id.as_str(), e.formato.as_str(), e.ritmo),
        (1, "quadro_continuo", "rgb_320x180", Some(15.0))
    );
    assert_eq!(a.funcoes.len(), 1);
    let f = &a.funcoes[0];
    assert_eq!(
        (f.n, f.id.as_str(), f.entrega.as_str()),
        (1, "detector", "caixas")
    );
    assert_eq!(
        (f.leitor.as_deref(), f.publica.as_deref()),
        (Some("nanodet_v8"), Some("media.detections"))
    );
    assert_eq!((f.nota, f.iou), (Some(0.35), Some(0.5)));

    // A mensagem: 8 bytes de cabeçalho e 12 de carga, uma entrega da função 1.
    let (cab, carga) = MENSAGEM.split_at(Cabecalho::BYTES);
    let c = Cabecalho::ler(cab.try_into().unwrap()).expect("o cabeçalho gravado pelo lado Python");
    assert_eq!(
        c,
        Cabecalho {
            tipo: Tipo::Entrega,
            canal: 1,
            tamanho: 12
        }
    );
    assert_eq!(carga, (0u8..12).collect::<Vec<_>>());
    // E escrito de volta, dá os mesmos bytes.
    assert_eq!(c.bytes(), cab);
}

#[test]
fn anuncio_de_endereco_publico_nao_vale() {
    for vale in [
        ip(10, 0, 0, 7),
        ip(172, 16, 5, 1),
        ip(172, 31, 255, 254),
        ip(192, 168, 1, 102),
        ip(127, 0, 0, 1),
    ] {
        assert!(estacao::ler_anuncio(ANUNCIO, vale).is_some(), "{vale}");
    }
    for nao_vale in [
        ip(8, 8, 8, 8),
        ip(172, 32, 0, 1),
        ip(192, 169, 1, 1),
        ip(11, 0, 0, 1),
    ] {
        assert!(
            estacao::ler_anuncio(ANUNCIO, nao_vale).is_none(),
            "{nao_vale}"
        );
    }
    assert!(
        estacao::ler_anuncio(ANUNCIO, "::1".parse().unwrap()).is_none(),
        "o anúncio é IPv4"
    );
}

#[test]
fn o_malformado_e_o_de_outra_versao_sao_ignorados() {
    let de = ip(192, 168, 1, 102);
    for ruim in [
        &b""[..],
        b"nao e json",
        b"[1,2]",
        br#"{"rms1_hailo":1,"quadros":47002,"painel":8080,"entradas":[],"funcoes":[]}"#,
        br#"{"rms1_hailo":2,"quadros":0,"painel":8080,"entradas":[],"funcoes":[]}"#,
        br#"{"rms1_hailo":2,"quadros":47002,"painel":8080,"entradas":[{"n":0,"id":"x","formato":"y"}],"funcoes":[]}"#,
        br#"{"rms1_hailo":2,"quadros":47002,"painel":8080,"entradas":[],"funcoes":[{"n":1,"id":"x"}]}"#,
    ] {
        assert!(estacao::ler_anuncio(ruim, de).is_none(), "{}", String::from_utf8_lossy(ruim));
    }
    // Sem função nenhuma, a estação está no ar com tudo desligado.
    let vazio = br#"{"rms1_hailo":2,"quadros":47002,"painel":8080,"entradas":[],"funcoes":[]}"#;
    assert!(estacao::ler_anuncio(vazio, de).is_some());
    // Cabeçalho de outra versão, de tipo desconhecido ou com carga grande demais é recusado com o motivo.
    assert!(
        Cabecalho::ler(&[1, 1, 0, 0, 0, 0, 0, 0])
            .unwrap_err()
            .contains("versão 1")
    );
    assert!(Cabecalho::ler(&[2, 9, 0, 0, 0, 0, 0, 0]).is_err());
    assert!(Cabecalho::ler(&[2, 1, 1, 0, 1, 0, 0, 0x40]).is_err());
}
