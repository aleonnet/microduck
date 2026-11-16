// rms1-venc: o codificador de hardware do S1 (Hantro H1), chamado pelas bibliotecas que a DJI usa, do lado Android do robô.
//
// O `rms1-mediad` roda no chroot Debian e não carrega biblioteca do lado Android; este programa é o pedaço que roda lá. Ele
// faz o que o `dji_camera` faz por `libduml_hal.so` (`p1_venc.c` e `p1_ienc.c`), com as mesmas chamadas, na mesma ordem:
//   H.264: H264EncInit, Get/SetPreProcessing, Get/SetRateCtrl, H264EncStrmStart, H264EncStrmEncode por quadro, H264EncStrmEnd,
//          H264EncRelease   (`libOMX.hantro.H1.video.encoder.so`)
//   JPEG:  JpegEncInit, JpegEncSetPictureSize, JpegEncEncode por quadro, JpegEncRelease
//          (`libOMX.hantro.H1.image.encoder.so`)
// A memória do quadro e a da saída têm de ter endereço físico: são pedidas à própria biblioteca (EWLInit, EWLMallocLinear).
//
// As estruturas abaixo NÃO vêm de um cabeçalho: vêm da leitura do firmware copiado do robô em 2026-11-01, feita em 2026-11-15.
// O deslocamento de cada campo é o que a biblioteca usa ao registrar o campo pelo nome (`H264EncStrmStart#`, `H264EncSetRateCtrl#`,
// `H264EncSetPreProcessing#`) e o que a DJI escreve em `p1_venc_open` (0x155b0), `p1_venc_encfrm` (0x146ec), `p1_ienc_open`
// (0x14118) e `p1_ienc_encfrm` (0x14004) de `libduml_hal.so`. A leitura está no laboratório, em
// out/2026-11-14-s1-midia/adendo/leitura-codificador.md.
//
// SÓ RODA COM O `dji_camera` PARADO (o robô ativado pelo `rms1d-bench.sh up`): o codificador é um só.
//
// Uso:
//   rms1-venc info                                              as versões que as bibliotecas dizem; não codifica nada
//   rms1-venc jpeg <quadro.nv12> <larg> <alt> <q 0..10> <voltas> <saida.jpg>
//   rms1-venc h264 <quadro.nv12> <larg> <alt> <bits/s> <quadros/s> <voltas> <saida.h264>
//   rms1-venc servir <socket>                                   o serviço: o rms1-mediad manda o quadro e recebe o codificado
//
// O serviço (`servir`). O socket é de domínio Unix, num caminho que os dois lados enxergam: `/data/debian/run/rms1-venc/venc.sock`
// daqui, `/run/rms1-venc/venc.sock` de dentro do chroot. Um cliente por vez. O quadro é sempre NV12 de 640 por 360.
//   pedido:   1 byte de comando, 4 de parâmetro, 4 de tamanho (little-endian), e a carga
//     1  quadro em H.264        carga: o quadro NV12. O primeiro depois de um fluxo novo é inteiro e leva os cabeçalhos; depois,
//                               um inteiro por segundo
//     2  quadro em JPEG         parâmetro: o nível, de 0 a 10; carga: o quadro NV12
//     3  fluxo novo de H.264    parâmetro: a taxa em Mb/s nos 16 bits de baixo (0 = o máximo do chip: o quantizador fixo no
//                               mínimo) e os quadros por segundo nos 16 de cima; sem carga
//   resposta: 1 byte de estado (0 = certo), 4 de tamanho, e a carga: o quadro codificado, ou o motivo em texto
// Com SIGTERM ele termina o fluxo, solta o chip e apaga o socket.
// O arquivo é NV12 (Y, depois UV intercalado), no tamanho dito, com um quadro ou com vários em seguida: cada volta codifica o
// quadro seguinte, e recomeça do primeiro quando o arquivo acaba. Com zero voltas, uma por quadro do arquivo. Sai o tempo de
// cada quadro (só a chamada do codificador, sem a leitura do arquivo). O JPEG de cada volta vai um atrás do outro na saída.
#include <dlfcn.h>
#include <errno.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>

typedef uint32_t u32;
typedef int32_t i32;

// ---- a camada de baixo (EWL), igual nas duas bibliotecas --------------------------------------------------------------------
typedef struct { u32 clientType; } EwlInicio;                       // EWLInit lê [0] e recusa acima de 4
typedef struct { u32 *virt; u32 barramento; u32 tamanho; } EwlMemoria; // EWLMallocLinear grava [0], [4] e [8]

// ---- H.264 ------------------------------------------------------------------------------------------------------------------
typedef struct {            // 0x28 bytes; p1_venc_open: nível 41, uma referência
    u32 streamType;         // 0x00  0: fluxo de bytes (com os códigos de início)
    u32 viewMode;           // 0x04
    u32 level;              // 0x08
    u32 width;              // 0x0c
    u32 height;             // 0x10
    u32 frameRateNum;       // 0x14
    u32 frameRateDenom;     // 0x18
    u32 scaledWidth;        // 0x1c
    u32 scaledHeight;       // 0x20
    u32 refFrameAmount;     // 0x24
} H264Config;

typedef struct {            // 0x3c bytes
    u32 origWidth;          // 0x00
    u32 origHeight;         // 0x04
    u32 xOffset;            // 0x08
    u32 yOffset;            // 0x0c
    u32 inputType;          // 0x10  1: YUV 4:2:0 com UV intercalado (NV12)
    u32 rotation;           // 0x14
    u32 videoStabilization; // 0x18
    u32 colorConversion[4]; // 0x1c  o tipo e cinco coeficientes de 16 bits
    u32 scaledOutput;       // 0x2c
    u32 resto[3];           // 0x30  interlacedFrame e o que vier depois: fica como a biblioteca devolve
} H264Entrada;

typedef struct {            // 0x44 bytes
    u32 pictureRc;          // 0x00
    u32 mbRc;               // 0x04
    u32 pictureSkip;        // 0x08
    i32 qpHdr;              // 0x0c
    u32 qpMin;              // 0x10
    u32 qpMax;              // 0x14
    u32 bitPerSecond;       // 0x18
    u32 hrd;                // 0x1c
    u32 hrdCpbSize;         // 0x20
    u32 gopLen;             // 0x24
    u32 resto[7];           // 0x28  intraQpDelta em diante: fica como a biblioteca devolve
} H264Taxa;

typedef struct {            // 0x3c bytes
    u32 extra[4];           // 0x00  extensão desta versão; a DJI manda zero no caminho comum
    u32 busLuma;            // 0x10
    u32 busChromaU;         // 0x14
    u32 busChromaV;         // 0x18
    u32 timeIncrement;      // 0x1c
    u32 *pOutBuf;           // 0x20
    u32 busOutBuf;          // 0x24
    u32 outBufSize;         // 0x28
    u32 codingType;         // 0x2c  0: quadro inteiro (intra); 1: previsto
    u32 busLumaStab;        // 0x30
    u32 ipf;                // 0x34  3: usa e renova a referência; 2: só renova
    u32 ltrf;               // 0x38
} H264Quadro;

typedef struct { u32 codingType; u32 streamSize; u32 resto[13]; } H264Saida; // 0x3c bytes; p1_venc_start lê o tamanho em [4]

// ---- JPEG -------------------------------------------------------------------------------------------------------------------
typedef struct {            // 0x60 bytes; p1_ienc_open
    u32 inputWidth;         // 0x00
    u32 inputHeight;        // 0x04
    u32 xOffset;            // 0x08
    u32 yOffset;            // 0x0c
    u32 codingWidth;        // 0x10
    u32 codingHeight;       // 0x14
    u32 restartInterval;    // 0x18
    u32 qLevel;             // 0x1c
    const uint8_t *qTableLuma;   // 0x20
    const uint8_t *qTableChroma; // 0x24
    u32 frameType;          // 0x28  1: YUV 4:2:0 com UV intercalado (NV12)
    u32 colorConversion[4]; // 0x2c
    u32 rotation;           // 0x3c
    u32 codingType;         // 0x40  0: o quadro inteiro
    u32 codingMode;         // 0x44  0: 4:2:0
    u32 unitsType;          // 0x48  a DJI manda 1
    u32 markerType;         // 0x4c  a DJI manda 1
    u32 xDensity;           // 0x50  a DJI manda 72
    u32 yDensity;           // 0x54
    u32 comLength;          // 0x58
    const uint8_t *pCom;    // 0x5c
} JpegConfig;

typedef struct {            // p1_ienc_encfrm escreve até 0x24; o resto vai zerado
    u32 frameHeader;        // 0x00
    u32 busLum;             // 0x04
    u32 busCb;              // 0x08
    u32 busCr;              // 0x0c
    const uint8_t *pLum;    // 0x10
    const uint8_t *pCb;     // 0x14
    const uint8_t *pCr;     // 0x18
    uint8_t *pOutBuf;       // 0x1c
    u32 busOutBuf;          // 0x20
    u32 outBufSize;         // 0x24
    u32 resto[6];
} JpegQuadro;

typedef struct { u32 jfifSize; u32 resto[7]; } JpegSaida; // p1_ienc_encfrm lê o tamanho em [0]

#define VIDEO "libOMX.hantro.H1.video.encoder.so"
#define IMAGEM "libOMX.hantro.H1.image.encoder.so"
#define SAIDA_BYTES (512 * 1024)

static double agora_ms(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000.0 + t.tv_nsec / 1e6;
}

static void *abrir(const char *nome) {
    void *b = dlopen(nome, RTLD_NOW);
    if (!b) { fprintf(stderr, "não abri %s: %s\n", nome, dlerror()); exit(2); }
    return b;
}

static void *achar(void *b, const char *nome) {
    void *f = dlsym(b, nome);
    if (!f) { fprintf(stderr, "a biblioteca não tem %s\n", nome); exit(2); }
    return f;
}

static int comparar(const void *a, const void *b) {
    double x = *(const double *)a, y = *(const double *)b;
    return x < y ? -1 : x > y;
}

static void resumo(const char *nome, double *ms, int n, u32 bytes) {
    qsort(ms, n, sizeof *ms, comparar);
    printf("%s: mediana %.2f ms, pior %.2f ms, %u bytes\n", nome, ms[n / 2], ms[n - 1], bytes);
}

// O arquivo dos quadros, e quantos ele tem.
static FILE *abrir_quadros(const char *caminho, u32 l, u32 a, int *quantos) {
    FILE *f = fopen(caminho, "rb");
    if (!f || fseek(f, 0, SEEK_END)) { fprintf(stderr, "não abri %s\n", caminho); exit(2); }
    *quantos = (int)(ftell(f) / (long)(l * a * 3 / 2));
    if (*quantos < 1) { fprintf(stderr, "%s não tem um quadro NV12 de %u por %u\n", caminho, l, a); exit(2); }
    return f;
}

// O quadro de número i (dando a volta no arquivo) copiado para a memória do codificador.
static void carregar(FILE *f, int i, int quantos, u32 l, u32 a, EwlMemoria *m) {
    u32 tamanho = l * a * 3 / 2;
    if (fseek(f, (long)(i % quantos) * (long)tamanho, SEEK_SET) || fread(m->virt, 1, tamanho, f) != tamanho) { fprintf(stderr, "não li o quadro %d\n", i); exit(2); }
}

static void gravar(const char *caminho, const void *dados, u32 n, const char *modo) {
    FILE *f = fopen(caminho, modo);
    if (!f || fwrite(dados, 1, n, f) != n) { fprintf(stderr, "não gravei %s\n", caminho); exit(2); }
    fclose(f);
}

static int info(void) {
    void *v = abrir(VIDEO), *i = abrir(IMAGEM);
    // As duas funções devolvem uma estrutura pequena por valor: no ARM de 32 bits, por um ponteiro escondido em r0.
    struct { u32 major, minor; } api;
    struct { u32 swBuild, hwBuild; } build;
    ((void (*)(void *))achar(v, "H264EncGetApiVersion"))(&api);
    printf("H.264: interface %u.%u\n", api.major, api.minor);
    ((void (*)(void *))achar(v, "H264EncGetBuild"))(&build);
    printf("H.264: programa %u, chip 0x%08x\n", build.swBuild, build.hwBuild);
    ((void (*)(void *))achar(i, "JpegEncGetApiVersion"))(&api);
    printf("JPEG: interface %u.%u\n", api.major, api.minor);
    ((void (*)(void *))achar(i, "JpegEncGetBuild"))(&build);
    printf("JPEG: programa %u, chip 0x%08x\n", build.swBuild, build.hwBuild);
    return 0;
}

static int jpeg(int argc, char **argv) {
    if (argc != 8) return 64;
    u32 l = atoi(argv[3]), a = atoi(argv[4]), q = atoi(argv[5]);
    int voltas = atoi(argv[6]);
    void *b = abrir(IMAGEM);
    const void *(*EWLInit)(EwlInicio *) = achar(b, "EWLInit");
    i32 (*EWLMallocLinear)(const void *, u32, EwlMemoria *) = achar(b, "EWLMallocLinear");
    void (*EWLFreeLinear)(const void *, EwlMemoria *) = achar(b, "EWLFreeLinear");
    i32 (*EWLRelease)(const void *) = achar(b, "EWLRelease");
    i32 (*Init)(const JpegConfig *, void **) = achar(b, "JpegEncInit");
    i32 (*SetPictureSize)(void *, const JpegConfig *) = achar(b, "JpegEncSetPictureSize");
    i32 (*Encode)(void *, const JpegQuadro *, JpegSaida *) = achar(b, "JpegEncEncode");
    i32 (*Release)(void *) = achar(b, "JpegEncRelease");

    EwlInicio inicio = { 3 }; // o cliente de JPEG
    const void *ewl = EWLInit(&inicio);
    if (!ewl) { fprintf(stderr, "EWLInit falhou\n"); return 1; }
    EwlMemoria entra = { 0 }, sai = { 0 };
    if (EWLMallocLinear(ewl, l * a * 3 / 2, &entra) || EWLMallocLinear(ewl, SAIDA_BYTES, &sai)) { fprintf(stderr, "sem memória do codificador\n"); return 1; }
    int quantos;
    FILE *quadros = abrir_quadros(argv[2], l, a, &quantos);
    if (voltas == 0) voltas = quantos;

    JpegConfig c;
    memset(&c, 0, sizeof c);
    c.inputWidth = c.codingWidth = l;
    c.inputHeight = c.codingHeight = a;
    c.qLevel = q;
    c.frameType = 1;
    c.unitsType = 1;
    c.markerType = 1;
    c.xDensity = c.yDensity = 72;
    void *inst = NULL;
    i32 r = Init(&c, &inst);
    if (r) { fprintf(stderr, "JpegEncInit: %d\n", r); return 1; }
    r = SetPictureSize(inst, &c);
    if (r) { fprintf(stderr, "JpegEncSetPictureSize: %d\n", r); Release(inst); return 1; }

    double *ms = calloc(voltas, sizeof *ms);
    JpegSaida s;
    u32 total = 0;
    gravar(argv[7], "", 0, "wb");
    for (int i = 0; i < voltas; i++) {
        carregar(quadros, i, quantos, l, a, &entra);
        JpegQuadro e;
        memset(&e, 0, sizeof e);
        memset(&s, 0, sizeof s);
        e.frameHeader = 1;
        e.busLum = entra.barramento;
        e.busCb = entra.barramento + l * a;
        e.busCr = e.busCb;
        e.pOutBuf = (uint8_t *)sai.virt;
        e.busOutBuf = sai.barramento;
        e.outBufSize = SAIDA_BYTES;
        double t = agora_ms();
        r = Encode(inst, &e, &s);
        ms[i] = agora_ms() - t;
        if (r != 1) { fprintf(stderr, "JpegEncEncode: %d na volta %d\n", r, i); Release(inst); return 1; }
        gravar(argv[7], sai.virt, s.jfifSize, "ab");
        total += s.jfifSize;
    }
    char nome[96];
    snprintf(nome, sizeof nome, "JPEG de hardware %ux%u nível %u, %d quadros, média por quadro", l, a, q, voltas);
    resumo(nome, ms, voltas, total / voltas);
    Release(inst);
    EWLFreeLinear(ewl, &entra);
    EWLFreeLinear(ewl, &sai);
    EWLRelease(ewl);
    return 0;
}

static int h264(int argc, char **argv) {
    if (argc != 9) return 64;
    u32 l = atoi(argv[3]), a = atoi(argv[4]), bps = atoi(argv[5]), fps = atoi(argv[6]);
    int voltas = atoi(argv[7]);
    void *b = abrir(VIDEO);
    const void *(*EWLInit)(EwlInicio *) = achar(b, "EWLInit");
    i32 (*EWLMallocLinear)(const void *, u32, EwlMemoria *) = achar(b, "EWLMallocLinear");
    void (*EWLFreeLinear)(const void *, EwlMemoria *) = achar(b, "EWLFreeLinear");
    i32 (*EWLRelease)(const void *) = achar(b, "EWLRelease");
    i32 (*Init)(const H264Config *, void **) = achar(b, "H264EncInit");
    i32 (*GetPre)(void *, H264Entrada *) = achar(b, "H264EncGetPreProcessing");
    i32 (*SetPre)(void *, const H264Entrada *) = achar(b, "H264EncSetPreProcessing");
    i32 (*GetTaxa)(void *, H264Taxa *) = achar(b, "H264EncGetRateCtrl");
    i32 (*SetTaxa)(void *, const H264Taxa *) = achar(b, "H264EncSetRateCtrl");
    i32 (*Start)(void *, const H264Quadro *, H264Saida *) = achar(b, "H264EncStrmStart");
    // Nesta versão a função leva mais dois argumentos, o aviso de fatia pronta e o dado dele: a DJI passa zero quando não o usa.
    i32 (*Encode)(void *, const H264Quadro *, H264Saida *, void *, void *) = achar(b, "H264EncStrmEncode");
    i32 (*End)(void *, const H264Quadro *, H264Saida *) = achar(b, "H264EncStrmEnd");
    i32 (*Release)(void *) = achar(b, "H264EncRelease");

    EwlInicio inicio = { 1 }; // o cliente de H.264
    const void *ewl = EWLInit(&inicio);
    if (!ewl) { fprintf(stderr, "EWLInit falhou\n"); return 1; }
    // A altura codificada sobe ao múltiplo de 16: a memória do quadro cobre essas linhas a mais, zeradas.
    u32 cheia = l * ((a + 15) & ~15u) * 3 / 2;
    EwlMemoria entra = { 0 }, sai = { 0 };
    if (EWLMallocLinear(ewl, cheia, &entra) || EWLMallocLinear(ewl, SAIDA_BYTES, &sai)) { fprintf(stderr, "sem memória do codificador\n"); return 1; }
    memset(entra.virt, 0x80, cheia);
    int quantos;
    FILE *quadros = abrir_quadros(argv[2], l, a, &quantos);
    if (voltas == 0) voltas = quantos;

    H264Config c;
    memset(&c, 0, sizeof c);
    c.level = 41;
    c.width = l;
    c.height = a;
    c.frameRateNum = fps;
    c.frameRateDenom = 1;
    c.refFrameAmount = 1;
    void *inst = NULL;
    i32 r = Init(&c, &inst);
    if (r) { fprintf(stderr, "H264EncInit: %d\n", r); return 1; }
    H264Entrada p;
    memset(&p, 0, sizeof p);
    if ((r = GetPre(inst, &p))) { fprintf(stderr, "H264EncGetPreProcessing: %d\n", r); Release(inst); return 1; }
    p.origWidth = l;
    p.origHeight = a;
    p.xOffset = p.yOffset = 0;
    p.inputType = 1;
    p.rotation = 0;
    p.videoStabilization = 0;
    p.scaledOutput = 0;
    if ((r = SetPre(inst, &p))) { fprintf(stderr, "H264EncSetPreProcessing: %d\n", r); Release(inst); return 1; }
    H264Taxa x;
    memset(&x, 0, sizeof x);
    if ((r = GetTaxa(inst, &x))) { fprintf(stderr, "H264EncGetRateCtrl: %d\n", r); Release(inst); return 1; }
    printf("taxa de fábrica: pictureRc %u, mbRc %u, qpHdr %d, qp de %u a %u, %u bits/s, gop %u\n", x.pictureRc, x.mbRc, x.qpHdr, x.qpMin, x.qpMax, x.bitPerSecond, x.gopLen);
    x.pictureRc = 1;
    x.pictureSkip = 0;
    x.bitPerSecond = bps;
    x.hrd = 0;
    x.gopLen = fps;
    if ((r = SetTaxa(inst, &x))) { fprintf(stderr, "H264EncSetRateCtrl: %d\n", r); Release(inst); return 1; }

    H264Quadro e;
    H264Saida s;
    memset(&e, 0, sizeof e);
    memset(&s, 0, sizeof s);
    e.pOutBuf = sai.virt;
    e.busOutBuf = sai.barramento;
    e.outBufSize = SAIDA_BYTES;
    if ((r = Start(inst, &e, &s))) { fprintf(stderr, "H264EncStrmStart: %d\n", r); Release(inst); return 1; }
    gravar(argv[8], sai.virt, s.streamSize, "wb");
    printf("cabeçalho do fluxo: %u bytes\n", s.streamSize);

    double *ms = calloc(voltas, sizeof *ms);
    u32 total = 0, maior = 0, inteiro = 0;
    for (int i = 0; i < voltas; i++) {
        int intra = i % (int)fps == 0; // um quadro inteiro por segundo: quem entra no meio espera no máximo isso
        carregar(quadros, i, quantos, l, a, &entra);
        memset(&s, 0, sizeof s);
        e.busLuma = entra.barramento;
        e.busChromaU = entra.barramento + l * a;
        e.busChromaV = e.busChromaU;
        e.timeIncrement = i ? 1 : 0;
        e.codingType = intra ? 0 : 1;
        e.ipf = intra ? 2 : 3;
        e.ltrf = 0;
        double t = agora_ms();
        r = Encode(inst, &e, &s, NULL, NULL);
        ms[i] = agora_ms() - t;
        if (r < 0) { fprintf(stderr, "H264EncStrmEncode: %d na volta %d\n", r, i); break; }
        gravar(argv[8], sai.virt, s.streamSize, "ab");
        total += s.streamSize;
        if (intra) inteiro = s.streamSize;
        if (s.streamSize > maior) maior = s.streamSize;
    }
    if (r >= 0) {
        memset(&s, 0, sizeof s);
        if (End(inst, &e, &s) == 0) gravar(argv[8], sai.virt, s.streamSize, "ab");
        char nome[96];
        snprintf(nome, sizeof nome, "H.264 de hardware %ux%u a %u bits/s e %u quadros/s", l, a, bps, fps);
        resumo(nome, ms, voltas, total / voltas);
        printf("quadro inteiro: %u bytes; o maior quadro: %u bytes; na média: %.2f Mb/s\n", inteiro, maior, total / (double)voltas * fps * 8 / 1e6);
    }
    Release(inst);
    EWLFreeLinear(ewl, &entra);
    EWLFreeLinear(ewl, &sai);
    EWLRelease(ewl);
    return r < 0;
}

// ---- o serviço ----------------------------------------------------------------------------------------------------------------
#define S_LARG 640u
#define S_ALT 360u
#define S_QUADRO (S_LARG * S_ALT * 3 / 2)
#define S_QP_MINIMO 10 // o mínimo de fábrica do chip (a "taxa de fábrica" que o modo h264 imprime): a melhor qualidade que ele dá

static volatile sig_atomic_t s_parar = 0;
static void s_sinal(int n) { (void)n; s_parar = 1; }

// O H.264: a biblioteca, a memória do chip, o codificador e o fluxo em curso.
static struct {
    const void *ewl;
    EwlMemoria entra, sai;
    void *inst;
    u32 fps, n;                 // os quadros por segundo do fluxo, e quantos quadros ele já tem
    uint8_t cab[256];           // os cabeçalhos do fluxo, que vão na frente do primeiro quadro
    u32 cab_n;
    i32 (*Init)(const H264Config *, void **);
    i32 (*GetPre)(void *, H264Entrada *);
    i32 (*SetPre)(void *, const H264Entrada *);
    i32 (*GetTaxa)(void *, H264Taxa *);
    i32 (*SetTaxa)(void *, const H264Taxa *);
    i32 (*Start)(void *, const H264Quadro *, H264Saida *);
    i32 (*Encode)(void *, const H264Quadro *, H264Saida *, void *, void *);
    i32 (*End)(void *, const H264Quadro *, H264Saida *);
    i32 (*Release)(void *);
} V;

// O JPEG: o codificador é refeito quando o nível muda.
static struct {
    const void *ewl;
    EwlMemoria entra, sai;
    void *inst;
    u32 nivel;
    i32 (*Init)(const JpegConfig *, void **);
    i32 (*SetPictureSize)(void *, const JpegConfig *);
    i32 (*Encode)(void *, const JpegQuadro *, JpegSaida *);
    i32 (*Release)(void *);
} J;

static char s_motivo[160];
static int s_falha(const char *o_que, i32 r) {
    snprintf(s_motivo, sizeof s_motivo, "%s: %d", o_que, (int)r);
    return -1;
}

// Abre a biblioteca e pede ao chip a memória do quadro e a da saída. Devolve a memória por `entra` e `sai`.
static int s_abrir(const char *nome, u32 cliente, u32 entra_bytes, void **lib, const void **ewl, EwlMemoria *entra, EwlMemoria *sai) {
    *lib = abrir(nome);
    const void *(*EWLInit)(EwlInicio *) = achar(*lib, "EWLInit");
    i32 (*EWLMallocLinear)(const void *, u32, EwlMemoria *) = achar(*lib, "EWLMallocLinear");
    EwlInicio inicio = { cliente };
    *ewl = EWLInit(&inicio);
    if (!*ewl) return s_falha("EWLInit", -1);
    if (EWLMallocLinear(*ewl, entra_bytes, entra) || EWLMallocLinear(*ewl, SAIDA_BYTES, sai)) return s_falha("a memória do codificador", -1);
    return 0;
}

static void s_h264_fechar(void) {
    if (!V.inst) return;
    H264Quadro e;
    H264Saida s;
    memset(&e, 0, sizeof e);
    memset(&s, 0, sizeof s);
    e.pOutBuf = V.sai.virt;
    e.busOutBuf = V.sai.barramento;
    e.outBufSize = SAIDA_BYTES;
    V.End(V.inst, &e, &s);
    V.Release(V.inst);
    V.inst = NULL;
}

// Um fluxo novo de H.264: o codificador é refeito, e o primeiro quadro sai inteiro, com os cabeçalhos na frente.
static int s_h264_fluxo(u32 mbps, u32 fps) {
    static void *lib = NULL;
    if (!lib) {
        if (s_abrir(VIDEO, 1, S_LARG * ((S_ALT + 15) & ~15u) * 3 / 2, &lib, &V.ewl, &V.entra, &V.sai)) return -1;
        memset(V.entra.virt, 0x80, S_LARG * ((S_ALT + 15) & ~15u) * 3 / 2);
        V.Init = achar(lib, "H264EncInit");
        V.GetPre = achar(lib, "H264EncGetPreProcessing");
        V.SetPre = achar(lib, "H264EncSetPreProcessing");
        V.GetTaxa = achar(lib, "H264EncGetRateCtrl");
        V.SetTaxa = achar(lib, "H264EncSetRateCtrl");
        V.Start = achar(lib, "H264EncStrmStart");
        V.Encode = achar(lib, "H264EncStrmEncode");
        V.End = achar(lib, "H264EncStrmEnd");
        V.Release = achar(lib, "H264EncRelease");
    }
    s_h264_fechar();
    if (fps < 1 || fps > 60) return s_falha("quadros por segundo fora de 1 a 60", (i32)fps);
    H264Config c;
    memset(&c, 0, sizeof c);
    c.level = 41;
    c.width = S_LARG;
    c.height = S_ALT;
    c.frameRateNum = fps;
    c.frameRateDenom = 1;
    c.refFrameAmount = 1;
    i32 r = V.Init(&c, &V.inst);
    if (r) { V.inst = NULL; return s_falha("H264EncInit", r); }
    H264Entrada p;
    memset(&p, 0, sizeof p);
    if ((r = V.GetPre(V.inst, &p))) return s_falha("H264EncGetPreProcessing", r);
    p.origWidth = S_LARG;
    p.origHeight = S_ALT;
    p.xOffset = p.yOffset = 0;
    p.inputType = 1;
    p.rotation = 0;
    p.videoStabilization = 0;
    p.scaledOutput = 0;
    if ((r = V.SetPre(V.inst, &p))) return s_falha("H264EncSetPreProcessing", r);
    H264Taxa x;
    memset(&x, 0, sizeof x);
    if ((r = V.GetTaxa(V.inst, &x))) return s_falha("H264EncGetRateCtrl", r);
    x.pictureSkip = 0;
    x.hrd = 0;
    x.gopLen = fps;
    if (mbps == 0) {
        // O máximo do chip: sem controle de taxa, e o quantizador fixo no mínimo em todo quadro, o primeiro inclusive.
        x.pictureRc = 0;
        x.mbRc = 0;
        x.qpHdr = S_QP_MINIMO;
        x.qpMin = S_QP_MINIMO;
    } else {
        x.pictureRc = 1;
        x.bitPerSecond = mbps * 1000000u;
    }
    if ((r = V.SetTaxa(V.inst, &x))) return s_falha("H264EncSetRateCtrl", r);
    H264Quadro e;
    H264Saida s;
    memset(&e, 0, sizeof e);
    memset(&s, 0, sizeof s);
    e.pOutBuf = V.sai.virt;
    e.busOutBuf = V.sai.barramento;
    e.outBufSize = SAIDA_BYTES;
    if ((r = V.Start(V.inst, &e, &s))) return s_falha("H264EncStrmStart", r);
    if (s.streamSize > sizeof V.cab) return s_falha("os cabeçalhos do fluxo não cabem", (i32)s.streamSize);
    memcpy(V.cab, V.sai.virt, s.streamSize);
    V.cab_n = s.streamSize;
    V.fps = fps;
    V.n = 0;
    return 0;
}

// Um quadro em H.264. A saída fica na memória do chip; o primeiro quadro de um fluxo leva os cabeçalhos em `cab`.
static int s_h264_quadro(const uint8_t *nv12, const uint8_t **cab, u32 *cab_n, const uint8_t **dados, u32 *n) {
    if (!V.inst) return s_falha("quadro em H.264 sem um fluxo aberto (o comando 3 vem antes)", 0);
    memcpy(V.entra.virt, nv12, S_QUADRO);
    const int inteiro = V.n % V.fps == 0;
    H264Quadro e;
    H264Saida s;
    memset(&e, 0, sizeof e);
    memset(&s, 0, sizeof s);
    e.busLuma = V.entra.barramento;
    e.busChromaU = V.entra.barramento + S_LARG * S_ALT;
    e.busChromaV = e.busChromaU;
    e.timeIncrement = V.n ? 1 : 0;
    e.pOutBuf = V.sai.virt;
    e.busOutBuf = V.sai.barramento;
    e.outBufSize = SAIDA_BYTES;
    e.codingType = inteiro ? 0 : 1;
    e.ipf = inteiro ? 2 : 3;
    i32 r = V.Encode(V.inst, &e, &s, NULL, NULL);
    if (r < 0) return s_falha("H264EncStrmEncode", r);
    *cab = V.cab;
    *cab_n = V.n == 0 ? V.cab_n : 0;
    *dados = (const uint8_t *)V.sai.virt;
    *n = s.streamSize;
    V.n++;
    return 0;
}

// Um quadro em JPEG, no nível dado.
static int s_jpeg_quadro(const uint8_t *nv12, u32 nivel, const uint8_t **dados, u32 *n) {
    static void *lib = NULL;
    if (!lib) {
        if (s_abrir(IMAGEM, 3, S_QUADRO, &lib, &J.ewl, &J.entra, &J.sai)) return -1;
        J.Init = achar(lib, "JpegEncInit");
        J.SetPictureSize = achar(lib, "JpegEncSetPictureSize");
        J.Encode = achar(lib, "JpegEncEncode");
        J.Release = achar(lib, "JpegEncRelease");
        J.nivel = 99;
    }
    if (nivel > 10) return s_falha("nível do JPEG fora de 0 a 10", (i32)nivel);
    if (!J.inst || J.nivel != nivel) {
        if (J.inst) J.Release(J.inst);
        J.inst = NULL;
        JpegConfig c;
        memset(&c, 0, sizeof c);
        c.inputWidth = c.codingWidth = S_LARG;
        c.inputHeight = c.codingHeight = S_ALT;
        c.qLevel = nivel;
        c.frameType = 1;
        c.unitsType = 1;
        c.markerType = 1;
        c.xDensity = c.yDensity = 72;
        i32 r = J.Init(&c, &J.inst);
        if (r) { J.inst = NULL; return s_falha("JpegEncInit", r); }
        if ((r = J.SetPictureSize(J.inst, &c))) { J.Release(J.inst); J.inst = NULL; return s_falha("JpegEncSetPictureSize", r); }
        J.nivel = nivel;
    }
    memcpy(J.entra.virt, nv12, S_QUADRO);
    JpegQuadro e;
    JpegSaida s;
    memset(&e, 0, sizeof e);
    memset(&s, 0, sizeof s);
    e.frameHeader = 1;
    e.busLum = J.entra.barramento;
    e.busCb = J.entra.barramento + S_LARG * S_ALT;
    e.busCr = e.busCb;
    e.pOutBuf = (uint8_t *)J.sai.virt;
    e.busOutBuf = J.sai.barramento;
    e.outBufSize = SAIDA_BYTES;
    i32 r = J.Encode(J.inst, &e, &s);
    if (r != 1) return s_falha("JpegEncEncode", r);
    *dados = (const uint8_t *)J.sai.virt;
    *n = s.jfifSize;
    return 0;
}

static int s_ler(int fd, void *b, size_t n) {
    size_t f = 0;
    while (f < n) {
        ssize_t r = read(fd, (char *)b + f, n - f);
        if (r <= 0) return -1;
        f += (size_t)r;
    }
    return 0;
}

static int s_escrever(int fd, const void *b, size_t n) {
    size_t f = 0;
    while (f < n) {
        ssize_t r = write(fd, (const char *)b + f, n - f);
        if (r <= 0) return -1;
        f += (size_t)r;
    }
    return 0;
}

static u32 s_u32(const uint8_t *b) { return (u32)b[0] | ((u32)b[1] << 8) | ((u32)b[2] << 16) | ((u32)b[3] << 24); }

static int s_responder(int fd, uint8_t estado, const uint8_t *a, u32 an, const uint8_t *b, u32 bn) {
    const u32 n = an + bn;
    uint8_t cab[5] = { estado, (uint8_t)n, (uint8_t)(n >> 8), (uint8_t)(n >> 16), (uint8_t)(n >> 24) };
    if (s_escrever(fd, cab, 5) || (an && s_escrever(fd, a, an)) || (bn && s_escrever(fd, b, bn))) return -1;
    return 0;
}

static int servir(const char *caminho) {
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = s_sinal; // sem SA_RESTART: o sinal interrompe a espera, e o laço vê `s_parar`
    sigaction(SIGTERM, &sa, NULL);
    sigaction(SIGINT, &sa, NULL);
    // O cliente que fecha no meio de uma resposta não derruba o serviço. Por `sigaction`: o sistema do robô (Android 4.4) não
    // exporta `signal`, e a conferência da compilação reprovou a primeira versão por isso.
    struct sigaction ignora;
    memset(&ignora, 0, sizeof ignora);
    ignora.sa_handler = SIG_IGN;
    sigaction(SIGPIPE, &ignora, NULL);
    struct sockaddr_un end;
    memset(&end, 0, sizeof end);
    end.sun_family = AF_UNIX;
    if (strlen(caminho) >= sizeof end.sun_path) { fprintf(stderr, "o caminho do socket é longo demais\n"); return 2; }
    strcpy(end.sun_path, caminho);
    int s = socket(AF_UNIX, SOCK_STREAM, 0);
    unlink(caminho);
    if (s < 0 || bind(s, (struct sockaddr *)&end, sizeof end) || listen(s, 1)) { fprintf(stderr, "não abri o socket %s: %s\n", caminho, strerror(errno)); return 1; }
    chmod(caminho, 0666);
    fprintf(stderr, "rms1-venc: servindo em %s\n", caminho);
    static uint8_t quadro[S_QUADRO];
    while (!s_parar) {
        int c = accept(s, NULL, NULL);
        if (c < 0) continue;
        uint8_t ped[9];
        while (!s_parar && s_ler(c, ped, 9) == 0) {
            const uint8_t comando = ped[0];
            const u32 param = s_u32(ped + 1), tamanho = s_u32(ped + 5);
            if (tamanho != 0 && tamanho != S_QUADRO) break; // sem confiar no tamanho, não há como achar o próximo pedido
            if (tamanho && s_ler(c, quadro, tamanho)) break;
            const uint8_t *cab = NULL, *dados = NULL;
            u32 cab_n = 0, n = 0;
            int r;
            if (comando == 3) r = s_h264_fluxo(param & 0xffff, param >> 16);
            else if (comando == 1 && tamanho) r = s_h264_quadro(quadro, &cab, &cab_n, &dados, &n);
            else if (comando == 2 && tamanho) r = s_jpeg_quadro(quadro, param, &dados, &n);
            else r = s_falha("pedido desconhecido, ou quadro sem carga", comando);
            if (r ? s_responder(c, 1, (const uint8_t *)s_motivo, (u32)strlen(s_motivo), NULL, 0) : s_responder(c, 0, cab, cab_n, dados, n)) break;
        }
        close(c);
    }
    s_h264_fechar();
    if (J.inst) J.Release(J.inst);
    close(s);
    unlink(caminho);
    fprintf(stderr, "rms1-venc: saiu\n");
    return 0;
}

int main(int argc, char **argv) {
    int r = 64;
    if (argc >= 2 && !strcmp(argv[1], "info")) r = info();
    else if (argc >= 2 && !strcmp(argv[1], "jpeg")) r = jpeg(argc, argv);
    else if (argc >= 2 && !strcmp(argv[1], "h264")) r = h264(argc, argv);
    else if (argc == 3 && !strcmp(argv[1], "servir")) r = servir(argv[2]);
    if (r == 64) fprintf(stderr, "uso: rms1-venc info | jpeg <quadro.nv12> <larg> <alt> <q> <voltas> <saida.jpg> | h264 <quadro.nv12> <larg> <alt> <bits/s> <quadros/s> <voltas> <saida.h264> | servir <socket>\n");
    return r;
}
