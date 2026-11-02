---
name: nolgia-ugc-ads
description: "Produce vertical (9:16) UGC-style ad videos on NOLGIA: persona spec, consistent character portraits, app-screenshot B-roll, a lip-synced talking head from a voiceover, and face-safe text overlay rules. Use when: the user wants TikTok, Reels or Shorts ads, testimonial or influencer-style content, a talking-head product pitch, or one character scaled across many ad variants. NOT for: cinematic short films or trailers (use nolgia-video-prompting), single images or plain clips without an ad structure (use nolgia-platform), or horizontal TV spots."
version: 1.11.0
author: NOLGIA
license: MIT
metadata:
  tags: [nolgia, ugc, ads, vertical-video, tiktok, reels, social]
  related_skills: [nolgia-platform, nolgia-video-prompting]
---

# UGC Ads on NOLGIA

The pipeline that makes a UGC ad feel *edited* rather than *generated*:
persona → assets → talking head → interleaved timeline → safe-zone text.
Every step has a gate — don't advance past broken artifacts.

## 1. Persona first (one page, before any generation)

Identity (name/age/profession), visual spec (exact wardrobe wording —
you'll repeat it verbatim in every prompt), one specific pain-point
narrative, the app where they'd actually message, voice/energy, and 3–5
screenshot scenarios specific to THEIR story. Generic persona ⇒ generic
ad. Never reuse screenshots across characters — it kills authenticity.

Write the visual spec the way the distilled creator work writes a person:
age as a number, job, build and height, hair, the outfit by colour, fabric
and wear, and at most one mark the story needs ("MARCUS, 38, a plumber, about
180 cm, broad build; close-cropped black hair, a short beard; a faded navy
work jacket over a grey t-shirt"). Never a list of skin flaws (pores, lines,
spots, shine), a skin clause or a "not a model" lock, and never beauty words
(flawless, dewy, glass smooth): the model draws every listed flaw at once and
the face reads as made.

## 2. Assets, in this order

```bash
# Portrait: head/shoulders in the UPPER 60% of frame (lower 40% = text space).
# Send --aura on every image call (omitted, the server adds its own skin and
# finish text to a person): --aura false, since the prompt writes its light.
# nano-banana-pro at 2k makes one image a job: run it 4 times (marcus-1..4)
# and save the one that fits the persona as marcus.png.
nolgia gen image --model nano-banana-pro --quality 2k --aura false \
  --aspect-ratio 9:16 --prompt "Photoreal. <persona visual spec>. LOOK: a \
candid photo taken on a high-quality phone camera, the phone's portrait lens \
(about 50 mm equivalent) held at arm's length: his face in focus, the room \
behind falling into a soft, natural background blur. Soft window light from \
one side with open shadows; natural, true-to-life colour; casual, unposed \
framing." --out marcus-1.png

# Screenshots (3-5, each one narrative-specific): words that must read go on
# gpt-image-2.5-flare, also with --aura sent
nolgia gen image --model gpt-image-2.5-flare --aura false \
  --prompt "phone messaging UI: unread 9pm quote request from a customer..." --out s1.png

# Voiceover: a TTS model plus a voice from its catalog (the default audio
# model makes music, not speech)
nolgia voices list --model fal-ai/elevenlabs/tts/eleven-v3
nolgia gen audio --model fal-ai/elevenlabs/tts/eleven-v3 --voice <voice id> \
  --prompt "<the 15s script>" --out vo.mp3
```

## 3. Talking-head clip: lip sync from the voiceover (9:16, set it explicitly)

```bash
nolgia gen video --model heygen-avatar-iv \
  --input marcus.png --audio-ref vo.mp3 --aspect-ratio 9:16 \
  --prompt "speaking to camera, natural expression, small head movements; \
handheld phone footage at arm's length with a small natural sway, focus \
holding on his face, the room softly blurred behind him; soft window light \
from one side" \
  --out marcus_talk.mp4
```

The phone look in every UGC prompt is the one above: a high-quality phone
camera, its portrait lens with a natural background blur, soft window light,
photoreal. A ring light stays an option where the creator format calls for
one (a beauty desk, a talking head at night); window light is the default.
Never "HDR", "HDR flattening", "digital smartphone sharpness", "deep focus",
"NO shallow depth of field" or "NO bokeh": none of the 1,294 people prompts in
the influencer work we studied writes them.

`heygen-avatar-iv` makes the portrait speak the voice track: the clip is as
long as the audio and billed on its duration, so leave `--duration-seconds`
off. Price a lip-sync batch from the catalog, not from `--cost-only`:
`nolgia models get heygen-avatar-iv` quotes a 5-second clip, so scale that by
each voiceover's length (`--cost-only` does not read `--audio-ref` and would
quote 5 seconds however long the track is). For a
performance with more body movement, generate a silent base clip on
`minimax-h3` (`--input marcus.png --duration-seconds 15`) and cut the
lip-synced shots into it; step 4's B-roll hides the joins.

## 4. The timeline (what makes it an ad)

Interleave: 4–5 talking-head segments (1–4s, varied) + 3–4 screenshot
B-roll inserts (1–1.5s, slight push-in zoom, fade in/out) + the voiceover
driving problem → solution → CTA. A single 15s clip with text on top is
NOT a UGC ad.

Assemble the ordered clips into one rendered cut with a composition — see
the `nolgia-platform` skill's "Assemble clips into ONE finished video":
`nolgia compositions create --clip … --clip … --render --wait` (vertical:
`--width 1080 --height 1920`). A project folder is not the ad; you have not
delivered until a render succeeds and you can name the finished asset.

## 5. Text overlay: face-safe zones (hard rule)

- **Top 16%** of frame: hook only ("I WAS LOSING $3K A MONTH")
- **Middle 68%**: the face — NEVER text
- **Bottom 16%**: captions/CTA ("NOLGIA.COM — SET UP IN 5 MIN")

Auto-fit text width to ≤92% of frame; shrink the font rather than
overflow; drop emoji (most display fonts render tofu); dark pill behind
white text for readability. Burn text in post — never ask the model to
render text.

## QA gates before shipping

1. Watch with sound OFF: does the story read from visuals + text alone?
2. Watch with sound ON: does any cut feel unmotivated? Add ambient/SFX.
3. Freeze random frames: text inside safe zones, face unobstructed?
4. Is the first 1.5s a hook (motion + claim), not a slow fade-in?
