//! O mixer do S1 contra um chip de mentira: a ordem e os valores da partida da DJI, a recusa do valor que não pega, e o
//! volume na conta da DJI. Os números esperados estão aqui por extenso, e não vêm das constantes do módulo.

use std::collections::HashMap;
use std::io;

use rms1_mediad::mixer::{self, Controles, Lido};

/// O chip de mentira: guarda o que foi gravado, com os nomes da listagem medida no robô.
#[derive(Default)]
struct Chip {
    valores: HashMap<u32, Vec<u32>>,
    gravados: Vec<(u32, Vec<u32>)>,
    /// O controle que ignora a gravação e fica no valor dado.
    preso: Option<(u32, Vec<u32>)>,
    /// O controle que responde com outro nome.
    outro_nome: Option<u32>,
}

fn nome(id: u32) -> &'static str {
    match id {
        0 => "DACR Input",
        1 => "DACL Input",
        2 => "Stereo DAC Mute",
        5 => "Speaker Volume",
        6 => "Headphone Volume",
        11 => "Stereo DAC Gain",
        12 => "ADC1 Gain",
        13 => "ADC2 Gain",
        14 => "MIC1 Volume",
        15 => "MIC2 Volume",
        19 => "MIC1 Boost",
        20 => "MIC2 Boost",
        21 => "Pcm To Codec",
        25 => "Adc2 Mixer Mic2 Input Switch",
        26 => "Adc1 Mixer Mic1 Input Switch",
        40 => "Aux Mixer Right Playback Switch",
        45 => "Spk Mixer Right Playback Switch",
        50 => "I2sR Out",
        51 => "I2sL Out",
        54 => "mic1Pga mux",
        55 => "sideTn mux",
        56 => "mic2Pga mux",
        _ => "controle que a DJI não grava",
    }
}

impl Controles for Chip {
    fn gravar(&mut self, id: u32, valores: &[u32]) -> io::Result<()> {
        self.gravados.push((id, valores.to_vec()));
        match &self.preso {
            Some((preso, fica)) if *preso == id => self.valores.insert(id, fica.clone()),
            _ => self.valores.insert(id, valores.to_vec()),
        };
        Ok(())
    }

    fn ler(&mut self, id: u32) -> io::Result<Lido> {
        let nome = if self.outro_nome == Some(id) {
            "Receiver Volume"
        } else {
            nome(id)
        };
        Ok(Lido {
            nome: nome.to_owned(),
            valores: self.valores.get(&id).cloned().unwrap_or_default(),
        })
    }
}

#[test]
fn mixer_grava_confere_e_recusa_calado() {
    // As 18 linhas do `start_dji_system.sh` e os seis controles do `dji_camera`, na ordem, como foram lidos no firmware.
    let esperado: Vec<(u32, Vec<u32>)> = vec![
        (0, vec![1]),
        (1, vec![0]),
        (2, vec![1, 0]),
        (5, vec![20]),
        (6, vec![25, 25]),
        (11, vec![103, 103]),
        (40, vec![1]),
        (45, vec![0]),
        (14, vec![35]),
        (15, vec![35]),
        (21, vec![1]),
        (25, vec![1]),
        (26, vec![1]),
        (50, vec![0]),
        (51, vec![1]),
        (54, vec![0]),
        (55, vec![0]),
        (56, vec![0]),
        (12, vec![103]),
        (13, vec![103]),
        (19, vec![0]),
        (20, vec![1]),
        (14, vec![14]),
        (15, vec![29]),
    ];
    let mut chip = Chip::default();
    mixer::configurar(&mut chip).expect("o chip aceita tudo");
    assert_eq!(chip.gravados, esperado);
    // O estado final do microfone é o medido com a DJI viva.
    assert_eq!(chip.valores[&14], vec![14]);
    assert_eq!(chip.valores[&15], vec![29]);
    assert_eq!(chip.valores[&20], vec![1]);

    // Um valor que não pega: o serviço diz qual, e não grava mais nada.
    let mut chip = Chip {
        preso: Some((26, vec![0])),
        ..Chip::default()
    };
    let erro = mixer::configurar(&mut chip).expect_err("o valor não pegou");
    assert!(erro.contains("Adc1 Mixer Mic1 Input Switch"), "{erro}");
    assert!(erro.contains("[1]") && erro.contains("[0]"), "{erro}");
    assert_eq!(
        chip.gravados.last(),
        Some(&(26, vec![1])),
        "parou no controle que falhou"
    );
    assert_eq!(chip.gravados.len(), 13);

    // Um controle com outro nome: a numeração não é a esperada, e nada mais é gravado.
    let mut chip = Chip {
        outro_nome: Some(5),
        ..Chip::default()
    };
    let erro = mixer::configurar(&mut chip).expect_err("o nome difere");
    assert!(
        erro.contains("Receiver Volume") && erro.contains("Speaker Volume"),
        "{erro}"
    );
    assert_eq!(chip.gravados.len(), 4);
}

#[test]
fn volume_e_o_da_dji() {
    // 57 dá os 72 medidos com a DJI viva; 100 dá 127; a conta trunca.
    for (v, ganho) in [(57u8, 72u32), (100, 127), (1, 1), (50, 63), (54, 68)] {
        let mut chip = Chip::default();
        mixer::volume(&mut chip, v).expect("o chip aceita");
        assert_eq!(
            chip.gravados,
            vec![(2, vec![0, 0]), (11, vec![ganho, ganho])],
            "volume {v}"
        );
    }
    // Zero é o mudo, e o ganho não é tocado.
    let mut chip = Chip::default();
    mixer::volume(&mut chip, 0).expect("o chip aceita");
    assert_eq!(chip.gravados, vec![(2, vec![1, 1])]);
    // Acima de 100 vale 100.
    assert_eq!(mixer::ganho_do_volume(200), 127);
    // O ganho que não pega é dito pelo nome.
    let mut chip = Chip {
        preso: Some((11, vec![72, 72])),
        ..Chip::default()
    };
    let erro = mixer::volume(&mut chip, 80).expect_err("não pegou");
    assert!(erro.contains("Stereo DAC Gain"), "{erro}");
}

#[test]
fn a_resposta_do_amixer_e_lida_como_ele_escreve() {
    // Três respostas copiadas da listagem do robô (`out/2026-10-31-poc2/audio/amixer_contents.txt`, no laboratório).
    let inteiro = "numid=12,iface=MIXER,name='Stereo DAC Gain'\n  ; type=INTEGER,access=rw---R--,values=2,min=0,max=127,step=0\n  : values=72,72\n  | dBscale-min=-76.50dB,step=0.75dB,mute=1\n";
    assert_eq!(
        mixer::ler_cget(inteiro),
        Some(Lido {
            nome: "Stereo DAC Gain".into(),
            valores: vec![72, 72]
        })
    );
    let chave = "numid=3,iface=MIXER,name='Stereo DAC Mute'\n  ; type=BOOLEAN,access=rw------,values=2\n  : values=off,on\n";
    assert_eq!(
        mixer::ler_cget(chave),
        Some(Lido {
            nome: "Stereo DAC Mute".into(),
            valores: vec![0, 1]
        })
    );
    let lista = "numid=52,iface=MIXER,name='I2sL Out'\n  ; type=ENUMERATED,access=rw------,values=1,items=4\n  ; Item #0 'ADC1'\n  ; Item #1 'ADC2'\n  ; Item #2 'PDML'\n  ; Item #3 'PDMR'\n  : values=1\n";
    assert_eq!(
        mixer::ler_cget(lista),
        Some(Lido {
            nome: "I2sL Out".into(),
            valores: vec![1]
        })
    );
    assert_eq!(
        mixer::ler_cget("amixer: Cannot find the given element\n"),
        None
    );
}
