#ifndef JIAN_IOS_SPIKE_H
#define JIAN_IOS_SPIKE_H

#ifdef __cplusplus
extern "C" {
#endif

typedef struct JianIosSpike JianIosSpike;

JianIosSpike *jian_ios_spike_create(void *ca_metal_layer);
int jian_ios_spike_draw_red(JianIosSpike *spike);
void jian_ios_spike_destroy(JianIosSpike *spike);

#ifdef __cplusplus
}
#endif

#endif
