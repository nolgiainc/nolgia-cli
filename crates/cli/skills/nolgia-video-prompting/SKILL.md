---
name: nolgia-video-prompting
description: "Directing craft for NOLGIA video generation: shot grammar, the breathing pattern, multi-shot sequencing, character/location consistency with reference images, per-shot sound design, and the iteration process (keep approved shots, regenerate only what failed). Use when writing video prompts or planning multi-clip productions on NOLGIA."
version: 1.0.0
author: NOLGIA
license: MIT
metadata:
  tags: [nolgia, video-prompting, directing, seedance, kling, multi-shot, filmmaking]
---

# NOLGIA Video Prompting

Distilled from producing complete short films on the platform. Two layers:
what to put in a prompt, and how to run a multi-clip production without
burning credits.

## Prompt anatomy (one shot)

```
<SHOT SIZE>. <subject + one action>. <environment, time, weather>.
<camera: lens/movement>. <film stock / grade>. AUDIO: <sound design>.
```

Example: *WIDE SHOT. A rust-flecked Subaru moves south on an empty
two-lane highway through flat farmland. Late afternoon, overcast gray sky.
Static camera, 35mm at T4.0, Kodak Vision3 500T, subtle grain. AUDIO: car
engine, wind, distant birds — natural ambient.*

Rules that survived production:
- **One action per shot.** Two actions = the model picks one or morphs.
- **Repeat exact wording** for wardrobe/props/locations in every prompt
  that shows them ("olive field jacket with tear at left elbow" —
  verbatim, every time). Paraphrasing = drift.
- **Describe action only in image-to-video** — the reference image already
  carries character and environment; re-describing them fights it.
- **No text in generated frames.** Models garble it; burn text in post.
- **≤3 characters per shot**, close-up references beat distant ones.

## The breathing pattern (multi-shot sequencing)

Every cut changes shot size: WIDE (establish) → MEDIUM (observe) →
CLOSE-UP (feel) → WIDE (release). Never two adjacent shots at the same
distance. Vary pacing: calm scenes 3–4s/shot, tense 1.5–2s, danger <1s.
After every discovery, a half-second reaction beat on the protagonist —
reaction shots ARE the story. Never cut directly into a key moment: show
approach → threshold → reaction → event.

```bash
nolgia gen video --model fal-ai/bytedance/seedance/v2/pro/text-to-video \
  --prompt "Cinematic 35mm horror, muted palette." --generate-audio true \
  --shot "5:WIDE SHOT. Abandoned radio station at dusk, one lit window.|wind, a loose sign creaks" \
  --shot "4:MEDIUM. Maya at the threshold, hand on the door, listening.|her breath, faint static inside" \
  --shot "3:CU. Her face as the static resolves into a voice.|static becomes a whispered name"
```

## Consistency: reference images first

Generate character portraits and location plates ONCE (`nolgia gen image`),
then anchor every clip: `--input maya.png` flips to the image-to-video
model automatically. One reference per generation; for multi-angle
characters, stitch a reference sheet into a single image. Consistency
drift is the #1 defect in multi-clip work — lock refs before writing a
single shot.

## Sound is written into the shot

Every shot carries an AUDIO direction (the `|audio` part of `--shot`, or
an `AUDIO:` sentence). Silence is a direction too ("total silence" is the
strongest cut in the toolbox). Audio-as-afterthought produces cuts that
feel aggressive and unmotivated.

## The iteration loop (protects credits and quality)

1. Draft cheap: Kling standard or `veo-3.1-fast`, 720p-tier thinking.
2. Review each clip: **KEEP / FIX / CUT** — write it down.
3. **Never regenerate a KEEP.** Regenerate only FIX shots, changing one
   variable at a time (prompt sentence, seed, or ref — not all three).
4. Reuse a fixed `--seed` when re-rolling composition so only your change
   moves.
5. Final pass on the premium model only for shots that earned it.

The classic failure: regenerating everything each round because feedback
touched some shots. Approved clips are assets — edit around them.

## Assemble the finished cut

Individual clips are not a film. Once the shots are approved, sequence them
into one rendered video with a Studio composition (see the `nolgia-platform`
skill): `nolgia compositions create --name <n> --clip <shot1> --clip <shot2>
… --render --wait`. Clips play in the order given, each clip's audio is kept
(`--mute` to drop it). Do not report the production done until that render
succeeds and you can name the finished asset — a project folder full of
clips is not the deliverable.
