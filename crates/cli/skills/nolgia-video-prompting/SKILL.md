---
name: nolgia-video-prompting
description: "Directing craft for NOLGIA video generation: the film standard (references with stated jobs, casting and styling for the role, location plates, one structured prompt per take, written cuts at the genre's rhythm, best of N takes, sound, typeset words, the realism finish), plus the breathing pattern, per-shot sound design and the keep-what-works iteration loop. Use when: writing or fixing video prompts, planning a multi-clip production, a short film, a series episode, a trailer, a commercial or a music video on NOLGIA, or deciding which shots to regenerate. NOT for: platform mechanics such as auth, flags, jobs and credits (use nolgia-platform), vertical UGC ad pipelines (use nolgia-ugc-ads), or still-image prompting."
version: 1.5.0
author: NOLGIA
license: MIT
metadata:
  tags: [nolgia, video-prompting, directing, seedance, minimax-h3, multi-shot, filmmaking, realism]
  related_skills: [nolgia-platform, nolgia-ugc-ads]
---

# NOLGIA Video Prompting

Distilled from the source records of professionally made AI films (movies, short films, music
videos, commercials, ads and trailers: every prompt, reference and setting they sent) and from
NOLGIA's own realism lab, which proved what makes generated live action read as shot on a cinema
camera or a DSLR. Two layers: what to put in a prompt, and how to run a production without
burning credits.

## Prompt anatomy (one simple clip)

```
<SHOT SIZE>. <subject + one action>. <environment, time, weather>.
<camera: field of view and move>. <the light, physically>. AUDIO: <sound design>, no music.
```

Example: *WIDE SHOT. A rust-flecked hatchback moves south on an empty
two-lane highway through flat farmland. Late afternoon, overcast gray sky.
Static camera, about 60 degrees of view at road height. Flat overcast light
from above, no sun, wet asphalt a shade darker than the verge. AUDIO: car
engine, wind, distant birds, no music.*

Rules that survived production:
- **One action per shot.** Two actions and the model picks one or morphs.
- **Repeat exact wording** for wardrobe, props and locations in every prompt
  that shows them ("olive field jacket with a tear at the left elbow",
  verbatim, every time). Paraphrasing drifts.
- **Describe what the lens does, never its brand.** No camera or lens brand
  stacks, no film stock names in the prompt, and never "cinematic, 8k, ultra
  detailed, hyperrealistic, masterpiece, award winning, perfect, beautiful
  bokeh, flawless, glass smooth": those words gave a centred, symmetric
  advert frame with teal and orange. Never describe lens artifacts as things
  in the frame ("streak flares", "barrel distortion" and "vignette corners"
  render as overlays). Grain and halation go on in post.
- **Never default to teal and orange**, neon or a one colour grade.
- **No text in generated frames.** Models garble it; typeset words in the edit.
  Printed props and gear (tickets, labels, dive masks, control panels) invite
  pseudo lettering: write them plain, keep them blank or out of focus.
- **Never name an object you do not want**, not even to deny it ("jacket free
  sleeves" put a jacket on her).
- **Three characters per shot at most**; close-up references beat distant ones.

## Films: the standard, not shot-by-shot prompts

A short film, series episode, trailer, commercial, skit or music video is never
made by sending each line of a shot list to `nolgia gen video`. That is how an
in-house short came out looking like AI (2026-10-31). The order that works:

1. **Story first**: logline, scene list, the script with every spoken line, the
   take plan. Ask once for the frame (21:9, 4:3, 16:9 or 9:16) and check it
   against the route (section "Frame and finish").
2. **Cast, style, products and locations as stills** (below). Check every still
   at full size before it becomes a reference.
3. **Write each take** as one structured prompt, as one of two take types, at the
   genre's rhythm.
4. **Shoot**: best of N on performance takes, draft cheap and promote, patch with
   video edit before a reroll.
5. **Edit** on the cuts the takes actually contain; lay the score or song; typeset
   every word.
6. **Finish and check**: the realism post chain, bits per pixel, palette spread,
   every cut judged alone.

### References, not keyframes

- **Every take carries 2 to 5 references, each with a stated job and what not to
  take from it** (`--element`, up to 9 on `seedance-2.5` and `minimax-h3`):
  "@Image1 is Mara's cast sheet: her face, build, hair and wet work jacket only;
  not the grey backdrop, the panel layout or the pose." "@Image2 is the harbour
  plate looking north: geography and light direction only; not the opening frame."
- **A written FIRST FRAME block** pins the opening picture: shot size, field of
  view, camera height and side, each person's position as x/y percent of the
  frame, which way they face, the action already in motion.
- **A keyframe still is conditional**: only for a take with no references (a true
  start frame), a pose that must be exact, a stylized locked camera shot, or a
  hero frame whose composition must be exact. With references on `seedance-2.5`
  or `minimax-h3` it rides as an element ("@Image1 is the opening frame of
  segment 1"), since both refuse a start frame together with reference media.
- **Grok takes its start frame alone.** On `grok-imagine-video-1.5` send `--input`
  with no `--element`: with elements it ignores the start frame. Its prompt
  limit is 4,096 characters and it has no 21:9.
- **No `--character-id` for story roles.** A bound character carries its canonical
  outfit and accessories into every render and overrides the written wardrobe.
  Dress the character into the role as a cast sheet and send the sheet.

### Casting and styling for the role

- **Cast the role, not a look**: role, age, build, class, job; the face in plain
  physical terms with its flaws (lines, uneven skin with visible pores, tired
  eyes). "Not a model" in every still and take. Characterful faces read real.
- **Aura off** (`--aura false`) for every live action casting still, sheet, plate
  and keyframe: Aura appends model beauty and makeup text, and that face rides
  into the video. Never write beauty words (flawless, poreless, glass smooth,
  lit from within, dewy, symmetrical). Stills on `seedream-v5-pro`, sheet and
  state edits on `nano-banana-pro`; never `flux-ultra` for people.
- **One composite sheet per role**: one image on mid grey (#808080), no labels: full
  body front and back with the heads masked, plus one large face panel, made by
  editing the approved cast still so the face holds. One face, one person per
  sheet. Never an expression grid as the identity reference.
- **Every state is an edit of the approved sheet**, saved as its own reference
  (wardrobe change, wet, injured, night): "this exact character sheet, but she is
  soaked through; change only the wetness".
- **Wardrobe and props follow the role and the scene**: would this person wear or
  carry this, here, today? Worn as the job makes them; "no makeup, no jewelry, no
  watch" unless the role has them. Style references guide taste; the styling must
  fit the piece.
- **Products get a sheet from the customer's real photos**: "one full front, one
  3/4 perspective and one closeup of all the label text of the product from
  @image1. No added text, just the 3 angles." Write the packaging phrase once and
  paste it word for word into every take, with a scale lock ("about two thirds of
  her face height") and a label lock ("the label faces camera and turns with the
  can; never redraw or re letter it").

### Locations

- **Empty plates, one per camera direction**; a reverse angle is its own plate;
  three quarter, never frontal.
- **Light and weather states are edits of the approved plate** (night, power cut,
  rain): a plate with the lamp on pulls the lamp on.
- **Browse cheap**: 6 to 8 cheap variants per location, keep the one whose light
  already tells the story. Dress it lived in, object by object; signs blank.

### One structured prompt per take, in a held order

STYLE (the film's fixed header for this lighting setup), SCENE CONTEXT,
REFERENCES (each with its job), KEY LOCKS (identity, wardrobe, product phrase,
headcount, no text: early, so a long prompt never loses them), LOCATION MAP,
FIRST FRAME, FORMAT (the cuts), OPTICS (field of view per segment), CAMERA,
ACTION TIMING, ACTING, SCRIPT, PHYSICS, LIGHTING (only the sources in the scene,
where each sits, what it falls on), AUDIO ("diegetic sound only, no music"),
LOCKS and a FORBIDDEN line (plastic skin, model looks, makeup or jewelry off
role, beauty light, teal and orange, lens flares, readable text, music).

- **ACTING**: the full block for a performance take (a scene direction, then per
  character motive, goal, obstacle, tactic, beats in seconds, END STATE; "do not
  play the emotion"); a light form for comedy, action and product takes (physical
  beats per segment plus an END STATE).
- **One STYLE header per film and per lighting setup**, pasted identically, then a
  contradiction check: a night interior never inherits a daylight header.
- **Length goes up to the model's limit**, `video.max_prompt_chars` on
  `nolgia models get <model> --json` (`seedance-2.5` 19,800, `minimax-h3` 7,000,
  `grok-imagine-video-1.5` 4,096). Structured prompts of 5,000 to 8,000
  characters worked best. Over the limit the request is refused up front; trim
  style prose first, never a lock, the acting or a line. A CLI build older than
  the per-model limits (0.2.35 among them) refuses anything over 4,000
  characters client side ("longer than 4000 characters"): send that take with
  `nolgia api POST /generate/video` and a JSON body instead.

### Two take types, at the genre's rhythm

- **Coverage takes, 12 to 30 s, with 3 to 6 written cuts**: "One continuous
  sequence, FOUR hard-cut segments, 14 s of picture. HARD CUT at 3.5, 7.0 and
  10.5 s." then one line per segment, each a new shot size, cuts between spoken
  lines. `seedance-2.5` runs to 30 s; `minimax-h3` and Grok to 15 s.
- **Single shots, 4 to 8 s**, for action, inserts, VFX and the product hero.
- **The multi-shot flag stays off** (no `--shot`): the cuts are written in the prompt.
- **Handles**: set the duration 1 to 2 s longer than the written timeline and
  write the final hold as the END STATE.
- **Rhythm**: dialogue drama 1 to 3 segments of 4 to 7.5 s (it holds); comedy and
  action 3 to 5 of 3 to 4 s; fights 9 to 15 of about 1 s in a 15 s take; music
  video story takes about 3 s a cut; commercials and ads a cut every 1.8 to 2 s.
- **Cut the edit on the cuts the take actually contains**, never on the written
  seconds (they drift by up to 1.7 s). Every take ends on its END STATE and the
  next starts from it.

### Takes

- **Best of N by re running the identical prompt**: 3 for a hero or performance
  take, keep the one that plays the script; 1 plus review for inserts and wides.
- **Draft cheap, then promote**: prove a hard take's prompt at `seedance-2.5`
  480p, then render the identical prompt at 720p (no seed on that lane, so the
  720p render is a new roll of a proven prompt).
- **Patch with video edit before re rolling**: `seedance-2.5` with `video_task`
  `edit` keeps everything and changes one named thing.
- **Alternates of the same beat may intercut** at detected cuts; a scene is never
  assembled from unrelated single generations.

### Sound

- **Takes say "no music"**; the score or song is laid in the edit. Music goes in a
  take only when it plays in the scene (a radio).
- **Dialogue is spoken natively in the take**: each line at its second with the
  speaker and a delivery note, one mouth per segment ("only these 3 lines; nobody
  else speaks; listeners' lips stay still"), a VOICE line per speaker. Spell tricky
  words phonetically (hyphenated syllables, stress in capitals); a line in another
  language is written in its native words with the translation in parentheses.
- **TODO(speech test, round 47): the speaking lane is pending.** Until its result
  lands, people who speak on screen go to `minimax-h3` (native lip-synced
  speech), and `seedance-2.5` speaks when its references are needed.
- **Translation subtitles are typeset** when a line is not in the audience's
  language; a film still gets no captions.

### Words on screen and titles

- **Every word is typeset in the edit and checked word for word** against the
  customer's text. The packshot leaves two thirds of the frame empty for type;
  words get a 2 s locked hold; end cards run 2.6 to 5 s.
- **Titles stay locked to the frame** unless match moved to a verified track; light
  effects animate light only; a credit roll moves a constant whole number of
  pixels per frame (4 to 6 px at 24 fps for 1080) and goes on after grain.

### Frame and finish

| Frame | `seedance-2.5` | `minimax-h3` | `grok-imagine-video-1.5` |
|---|---|---|---|
| 21:9 | 480p and 720p (1080p refused) | yes | no: compose 16:9 for the crop |
| 4:3 | 480p and 720p | yes | yes |
| 16:9, 9:16 | 480p, 720p, 1080p | yes | yes |

`seedream-v5-pro` stills have no 21:9 (use `nano-banana-pro`). Re-read
`nolgia models get <model>` before quoting a frame.

- **The live action lane is `seedance-2.5` at 720p with references.** `minimax-h3`
  at 2k delivers 0.04 to 0.07 bits per pixel, which smears skin into wax: never
  use it for a shot where nobody speaks. Avoid `veo-3.1-fast` for people who must
  read as real.
- **The realism post chain, in this order**: keep the take at 720p; Topaz to 1080p
  with `topaz-proteus-natural` (split long takes at their cuts, since the restore
  window is about 7.4 s); grade with a negative stock at half strength or less
  (`kodak-vision3-250d` day, `kodak-vision3-500t` tungsten or sodium night); film
  emulation (halation, slight softness, grain) LAST at 0.6 strength, at delivery
  resolution, never before the upscale. The `kodak-2383-print` finish is not the
  default: at half strength it turns shadows cyan and crushes blacks.
- **Automatic checks** on every raw take and the master: bits per pixel (bitrate
  divided by width x height x fps; under 0.12 is starved), and palette spread (hue
  entropy under 2.0 bits over 36 bins, or a teal share over 0.40, flags a one
  colour or default teal look). A skin texture number is no gate; judge skin by
  eye at full size.

## The breathing pattern (multi-shot sequencing)

Every cut changes shot size: WIDE (establish), MEDIUM (observe), CLOSE-UP
(feel), WIDE (release). Never two adjacent shots at the same distance. After
every discovery, a half-second reaction beat on the protagonist: reaction shots
ARE the story. Never cut directly into a key moment: show approach, threshold,
reaction, event.

```bash
nolgia gen video --model seedance-2.5 --quality 720p --duration-seconds 14 \
  --generate-audio true --element <cast sheet uuid> --element <plate uuid> \
  --prompt "$(cat take-03.txt)"   # FORMAT: FOUR hard-cut segments, 12 s of picture,
                                  # HARD CUT at 3.0, 6.5, 9.0 s; WIDE, MEDIUM, CU, WIDE
```

## Sound is written into the shot

Every shot carries an AUDIO direction: the diegetic sounds by second and "no
music". Silence is a direction too ("total silence" is the strongest cut in the
toolbox). Audio as an afterthought produces cuts that feel aggressive and
unmotivated.

## The iteration loop (protects credits and quality)

1. Draft cheap: the lane's lowest tier (`seedance-2.5` 480p for a film take;
   compare prices with `nolgia models list --modality video` or `--cost-only`).
2. Review each clip at full size against the script: **KEEP / FIX / CUT**, written
   down.
3. **Never regenerate a KEEP.** Fix a FIX shot by video edit first, or regenerate
   it changing one variable at a time (a prompt sentence or a reference, not both).
4. On models that take `--seed` (`nolgia models get <model>`), reuse it when
   re-rolling composition so only your change moves.
5. Final pass on the keeper tier only for shots that earned it.

The classic failure: regenerating everything each round because feedback touched
some shots. Approved clips are assets: edit around them.

## Assemble the finished cut

Individual clips are not a film. Once the shots are approved, sequence them into
one rendered video with a Studio composition (see the `nolgia-platform` skill):
`nolgia compositions create --name <n> --clip <shot1> --clip <shot2> --render
--wait`. Clips play in the order given, each clip's audio is kept (`--mute` to drop
it). Do not report the production done until that render succeeds and you can
name the finished asset: a project folder full of clips is not the deliverable.
