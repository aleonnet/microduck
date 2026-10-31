/* The V4L2 UAPI as the ARM 32-bit compiler sees it: sizes, offsets, ioctl numbers and the
 * constants s1-cam uses, printed as JSON. Built and run by v4l2-oracle.sh inside a
 * linux/arm/v7 Debian bookworm container (glibc 2.36, 32-bit time_t — the layout the S1's kernel
 * 3.10 expects; strace names the buffer ioctls *_TIME32 for the same reason). The Rust side
 * (s1d/src/v4l2.rs) is held to this file's output in a test; nothing there is pinned by hand. */
#include <stdio.h>
#include <stddef.h>
#include <sys/time.h>
#include <linux/videodev2.h>

#define SZ(t) printf("  \"sizeof_%s\": %zu,\n", #t, sizeof(struct t))
#define OFF(t, f) printf("  \"off_%s_%s\": %zu,\n", #t, #f, offsetof(struct t, f))
#define IOC(n) printf("  \"%s\": %lu,\n", #n, (unsigned long)(n))

int main(void) {
    printf("{\n");
    printf("  \"sizeof_timeval\": %zu,\n", sizeof(struct timeval));
    SZ(v4l2_capability); OFF(v4l2_capability, driver); OFF(v4l2_capability, card);
    OFF(v4l2_capability, bus_info); OFF(v4l2_capability, version); OFF(v4l2_capability, capabilities);
    SZ(v4l2_input); OFF(v4l2_input, index); OFF(v4l2_input, name); OFF(v4l2_input, type);
    SZ(v4l2_control); OFF(v4l2_control, id); OFF(v4l2_control, value);
    SZ(v4l2_cropcap); OFF(v4l2_cropcap, type); OFF(v4l2_cropcap, bounds); OFF(v4l2_cropcap, defrect);
    OFF(v4l2_cropcap, pixelaspect);
    SZ(v4l2_format); OFF(v4l2_format, type); OFF(v4l2_format, fmt);
    SZ(v4l2_pix_format); OFF(v4l2_pix_format, width); OFF(v4l2_pix_format, height);
    OFF(v4l2_pix_format, pixelformat); OFF(v4l2_pix_format, field); OFF(v4l2_pix_format, bytesperline);
    OFF(v4l2_pix_format, sizeimage); OFF(v4l2_pix_format, colorspace); OFF(v4l2_pix_format, priv);
    SZ(v4l2_requestbuffers); OFF(v4l2_requestbuffers, count); OFF(v4l2_requestbuffers, type);
    OFF(v4l2_requestbuffers, memory);
    SZ(v4l2_buffer); OFF(v4l2_buffer, index); OFF(v4l2_buffer, type); OFF(v4l2_buffer, bytesused);
    OFF(v4l2_buffer, flags); OFF(v4l2_buffer, field); OFF(v4l2_buffer, timestamp); OFF(v4l2_buffer, timecode);
    OFF(v4l2_buffer, sequence); OFF(v4l2_buffer, memory); OFF(v4l2_buffer, m); OFF(v4l2_buffer, length);
    IOC(VIDIOC_QUERYCAP); IOC(VIDIOC_ENUMINPUT); IOC(VIDIOC_G_INPUT); IOC(VIDIOC_S_INPUT);
    IOC(VIDIOC_CROPCAP); IOC(VIDIOC_G_CTRL); IOC(VIDIOC_S_CTRL); IOC(VIDIOC_G_FMT); IOC(VIDIOC_S_FMT);
    IOC(VIDIOC_REQBUFS); IOC(VIDIOC_QUERYBUF); IOC(VIDIOC_QBUF); IOC(VIDIOC_DQBUF);
    IOC(VIDIOC_STREAMON); IOC(VIDIOC_STREAMOFF);
    IOC(V4L2_BUF_TYPE_VIDEO_CAPTURE); IOC(V4L2_MEMORY_MMAP); IOC(V4L2_PIX_FMT_NV12);
    IOC(V4L2_FIELD_ANY); IOC(V4L2_FIELD_INTERLACED); IOC(V4L2_COLORSPACE_JPEG);
    IOC(V4L2_CTRL_CLASS_CAMERA); IOC(V4L2_CAP_VIDEO_CAPTURE); IOC(V4L2_CAP_STREAMING);
    IOC(V4L2_INPUT_TYPE_CAMERA); IOC(V4L2_BUF_FLAG_MAPPED); IOC(V4L2_BUF_FLAG_QUEUED);
    IOC(V4L2_BUF_FLAG_DONE);
    printf("  \"arch\": \"arm32\"\n}\n");
    return 0;
}
