/* Direct libavcodec AAC-LC encoder; no subprocess or FFmpeg CLI. */
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <libavcodec/avcodec.h>
#include <libavutil/channel_layout.h>
#include <libavutil/error.h>

typedef struct {
    AVCodecContext *ctx;
    AVFrame *frame;
    AVPacket *packet;
} T3Aac;

void t3aac_free(T3Aac *s) {
    if (!s) return;
    av_packet_free(&s->packet);
    av_frame_free(&s->frame);
    avcodec_free_context(&s->ctx);
    free(s);
}

T3Aac *t3aac_open(int bitrate) {
    const AVCodec *codec = avcodec_find_encoder(AV_CODEC_ID_AAC);
    if (!codec) return NULL;
    T3Aac *s = calloc(1, sizeof(*s));
    if (!s) return NULL;
    s->ctx = avcodec_alloc_context3(codec);
    s->frame = av_frame_alloc();
    s->packet = av_packet_alloc();
    if (!s->ctx || !s->frame || !s->packet) goto fail;
    s->ctx->sample_rate = 48000;
    s->ctx->sample_fmt = AV_SAMPLE_FMT_FLTP;
    s->ctx->bit_rate = bitrate;
    s->ctx->profile = FF_PROFILE_AAC_LOW;
    s->ctx->time_base = (AVRational){1, 48000};
    s->ctx->flags |= AV_CODEC_FLAG_GLOBAL_HEADER;
    av_channel_layout_default(&s->ctx->ch_layout, 2);
    if (avcodec_open2(s->ctx, codec, NULL) < 0 || s->ctx->frame_size != 1024) goto fail;
    s->frame->nb_samples = 1024;
    s->frame->format = s->ctx->sample_fmt;
    s->frame->sample_rate = 48000;
    if (av_channel_layout_copy(&s->frame->ch_layout, &s->ctx->ch_layout) < 0 ||
        av_frame_get_buffer(s->frame, 0) < 0) goto fail;
    return s;
fail:
    t3aac_free(s);
    return NULL;
}

int t3aac_config(T3Aac *s, uint8_t *out, int capacity) {
    int n = s->ctx->extradata_size;
    if (n < 2 || n > capacity || !s->ctx->extradata) return -1;
    memcpy(out, s->ctx->extradata, n);
    return n;
}

int t3aac_send(T3Aac *s, const int16_t *pcm, int64_t pts) {
    if (!pcm) return avcodec_send_frame(s->ctx, NULL);
    int err = av_frame_make_writable(s->frame);
    if (err < 0) return err;
    for (int ch = 0; ch < 2; ++ch) {
        float *plane = (float *)s->frame->data[ch];
        for (int i = 0; i < 1024; ++i) plane[i] = pcm[2*i + ch] / 32768.0f;
    }
    s->frame->pts = pts;
    return avcodec_send_frame(s->ctx, s->frame);
}

/* 0 = needs input or drained, positive = packet size, negative = failure. */
int t3aac_receive(T3Aac *s, uint8_t *out, int capacity, int64_t *pts) {
    int err = avcodec_receive_packet(s->ctx, s->packet);
    if (err == AVERROR(EAGAIN) || err == AVERROR_EOF) return 0;
    if (err < 0) return err;
    int n = s->packet->size;
    if (n <= 0 || n > capacity || s->packet->pts == AV_NOPTS_VALUE) {
        av_packet_unref(s->packet);
        return -1;
    }
    *pts = s->packet->pts;
    memcpy(out, s->packet->data, n);
    av_packet_unref(s->packet);
    return n;
}
