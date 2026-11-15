// SPDX-License-Identifier: GPL-2.0-only
/*
 * Sound card for the Digium/Sangoma D-series desk phones (D40, D45, D50,
 * D60, D62, D65, D70) built around the Broadcom BCMRING "APM" audio front
 * end.
 *
 * The APM is CPU DAI and converter in one component, so the DAI links use
 * the dummy codec.  The phones have three transducer sets on two APM
 * channels: the handset on channel 0, and the headset and the speakerphone
 * on channel 1, told apart by the microphone input and output driver they
 * are wired to (and by the speaker amplifier enable line).  Each set gets
 * its own PCM device.  Opening a PCM enables the board pins of its set, and
 * DAPM then powers exactly the matching path of the APM; the wiring comes
 * from "audio-routing" in the devicetree.  The two PCMs that share channel
 * 1 exclude each other.
 *
 * On the D40 and D50 the headset and the speakerphone share the same
 * microphone input and output driver (channel 1, port A).  There the vendor
 * application additionally drives one GPIO, which it labels "spkr", whenever
 * the audio device changes: 1 for the speakerphone, 0 for the headset, and
 * no change for the handset.  The optional "speaker-select-gpios" property
 * reproduces that.  The values are those of the vendor binary; what the
 * line does electrically (an analog switch, a headset amplifier mute, or a
 * second loudspeaker amplifier enable) is not known, and neither is any
 * required order against the speaker amplifier enable.
 */

#include <linux/gpio/consumer.h>
#include <linux/mod_devicetable.h>
#include <linux/module.h>
#include <linux/of.h>
#include <linux/platform_device.h>

#include <sound/pcm.h>
#include <sound/soc.h>
#include <sound/soc-dapm.h>

/* PCM device order of the vendor's ALSA card. */
enum {
	DIGIUM_SPEAKERPHONE,
	DIGIUM_HANDSET,
	DIGIUM_HEADSET,
	DIGIUM_NUM_LINKS,
};

struct digium_path {
	const char *name;
	const char *cpu_dai;
	const char *mic;
	const char *out;
};

static const struct digium_path digium_paths[DIGIUM_NUM_LINKS] = {
	[DIGIUM_SPEAKERPHONE] = {
		"Speakerphone", "apm-ch1", "Speakerphone Mic", "Loudspeaker",
	},
	[DIGIUM_HANDSET] = {
		"Handset", "apm-ch0", "Handset Mic", "Handset Receiver",
	},
	[DIGIUM_HEADSET] = {
		"Headset", "apm-ch1", "Headset Mic", "Headset Receiver",
	},
};

struct digium_card {
	struct snd_soc_card card;
	struct snd_soc_dai_link links[DIGIUM_NUM_LINKS];
	struct snd_soc_dai_link_component cpus[DIGIUM_NUM_LINKS];
	struct snd_soc_dai_link_component platforms[DIGIUM_NUM_LINKS];
	/* Open substreams per link; serialized by the card's PCM mutex. */
	unsigned int users[DIGIUM_NUM_LINKS];
	/* D40, D50 only: speakerphone (1) versus headset (0) */
	struct gpio_desc *spk_sel;
};

static void digium_set_pins(struct snd_soc_card *card,
			    const struct digium_path *path, bool enable)
{
	struct snd_soc_dapm_context *dapm = snd_soc_card_to_dapm(card);

	if (enable) {
		snd_soc_dapm_enable_pin(dapm, path->mic);
		snd_soc_dapm_enable_pin(dapm, path->out);
	} else {
		snd_soc_dapm_disable_pin(dapm, path->mic);
		snd_soc_dapm_disable_pin(dapm, path->out);
	}
	snd_soc_dapm_sync(dapm);
}

static int digium_startup(struct snd_pcm_substream *substream)
{
	struct snd_soc_pcm_runtime *rtd = snd_soc_substream_to_rtd(substream);
	struct digium_card *priv = snd_soc_card_get_drvdata(rtd->card);
	unsigned int id = rtd->dai_link->id;
	unsigned int i;

	/* One transducer set at a time on an APM channel. */
	for (i = 0; i < DIGIUM_NUM_LINKS; i++)
		if (i != id && priv->users[i] &&
		    !strcmp(digium_paths[i].cpu_dai, digium_paths[id].cpu_dai))
			return -EBUSY;

	if (!priv->users[id]++) {
		/*
		 * As the vendor does: set on a change to the speakerphone,
		 * clear on a change to the headset, leave alone for the
		 * handset.  The line is set before the pins are enabled, so
		 * that it is settled when the speaker amplifier comes up
		 * (the vendor's two controls live in different places and
		 * have no fixed order).
		 */
		if (priv->spk_sel && id != DIGIUM_HANDSET)
			gpiod_set_value_cansleep(priv->spk_sel,
						 id == DIGIUM_SPEAKERPHONE);
		digium_set_pins(rtd->card, &digium_paths[id], true);
	}

	return 0;
}

static void digium_shutdown(struct snd_pcm_substream *substream)
{
	struct snd_soc_pcm_runtime *rtd = snd_soc_substream_to_rtd(substream);
	struct digium_card *priv = snd_soc_card_get_drvdata(rtd->card);
	unsigned int id = rtd->dai_link->id;

	if (!--priv->users[id])
		digium_set_pins(rtd->card, &digium_paths[id], false);
}

static const struct snd_soc_ops digium_ops = {
	.startup = digium_startup,
	.shutdown = digium_shutdown,
};

static const struct snd_soc_dapm_widget digium_widgets[] = {
	SND_SOC_DAPM_MIC("Speakerphone Mic", NULL),
	SND_SOC_DAPM_SPK("Loudspeaker", NULL),
	SND_SOC_DAPM_MIC("Handset Mic", NULL),
	SND_SOC_DAPM_HP("Handset Receiver", NULL),
	SND_SOC_DAPM_MIC("Headset Mic", NULL),
	SND_SOC_DAPM_HP("Headset Receiver", NULL),
};

static int digium_late_probe(struct snd_soc_card *card)
{
	unsigned int i;

	/* Nothing is connected until a PCM is opened. */
	for (i = 0; i < DIGIUM_NUM_LINKS; i++)
		digium_set_pins(card, &digium_paths[i], false);

	return 0;
}

static void digium_put_node(void *data)
{
	of_node_put(data);
}

static int digium_probe(struct platform_device *pdev)
{
	struct device *dev = &pdev->dev;
	struct device_node *cpu_np;
	struct digium_card *priv;
	struct snd_soc_card *card;
	unsigned int i;
	int ret;

	priv = devm_kzalloc(dev, sizeof(*priv), GFP_KERNEL);
	if (!priv)
		return -ENOMEM;

	card = &priv->card;
	card->dev = dev;
	card->owner = THIS_MODULE;
	card->dai_link = priv->links;
	card->num_links = DIGIUM_NUM_LINKS;
	card->dapm_widgets = digium_widgets;
	card->num_dapm_widgets = ARRAY_SIZE(digium_widgets);
	card->late_probe = digium_late_probe;
	/*
	 * Only the APM pins named in audio-routing are wired on a board; the
	 * others must not count as live inputs and outputs.
	 */
	card->fully_routed = true;
	snd_soc_card_set_drvdata(card, priv);

	ret = snd_soc_of_parse_card_name(card, "model");
	if (ret)
		return ret;

	ret = snd_soc_of_parse_audio_routing(card, "audio-routing");
	if (ret)
		return ret;

	ret = snd_soc_of_parse_aux_devs(card, "aux-devs");
	if (ret)
		return ret;

	/*
	 * Low is the level read on an idle D40 under the stock firmware.  The
	 * vendor drives the line high for an instant before each change; that
	 * is not reproduced.
	 */
	priv->spk_sel = devm_gpiod_get_optional(dev, "speaker-select",
						GPIOD_OUT_LOW);
	if (IS_ERR(priv->spk_sel))
		return dev_err_probe(dev, PTR_ERR(priv->spk_sel),
				     "bad speaker-select GPIO\n");

	cpu_np = of_parse_phandle(dev->of_node, "audio-cpu", 0);
	if (!cpu_np)
		return dev_err_probe(dev, -EINVAL, "missing audio-cpu\n");
	/* The links refer to the node for as long as the card exists. */
	ret = devm_add_action_or_reset(dev, digium_put_node, cpu_np);
	if (ret)
		return ret;

	for (i = 0; i < DIGIUM_NUM_LINKS; i++) {
		struct snd_soc_dai_link *link = &priv->links[i];

		priv->cpus[i].of_node = cpu_np;
		priv->cpus[i].dai_name = digium_paths[i].cpu_dai;
		priv->platforms[i].of_node = cpu_np;

		link->name = digium_paths[i].name;
		link->stream_name = digium_paths[i].name;
		link->id = i;
		link->cpus = &priv->cpus[i];
		link->num_cpus = 1;
		link->codecs = &snd_soc_dummy_dlc;
		link->num_codecs = 1;
		link->platforms = &priv->platforms[i];
		link->num_platforms = 1;
		link->ops = &digium_ops;
		/*
		 * Two links share an APM channel: a delayed power-down of
		 * one must not hit the other after a quick switch.
		 */
		link->ignore_pmdown_time = 1;
	}

	ret = devm_snd_soc_register_card(dev, card);
	if (ret)
		return dev_err_probe(dev, ret, "failed to register card\n");

	return 0;
}

static const struct of_device_id digium_of_match[] = {
	{ .compatible = "digium,bcmring-phone-audio" },
	{ }
};
MODULE_DEVICE_TABLE(of, digium_of_match);

static struct platform_driver digium_driver = {
	.probe = digium_probe,
	.driver = {
		.name = "bcmring-digium-card",
		.of_match_table = digium_of_match,
	},
};
module_platform_driver(digium_driver);

MODULE_DESCRIPTION("Digium D-series phone sound card (BCMRING APM)");
MODULE_LICENSE("GPL");
