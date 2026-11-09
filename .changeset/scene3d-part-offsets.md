---
"leptoskit_3d": minor
---

# Move a model's own parts from your app

`Scene::set_part_offset(node, part, offset, seconds)` (`Op::SetPartOffset`) moves a named part of a model instance, a bone or any node, from where its rest pose or the playing clip puts it. The offset turns the part about its own origin and moves it along its own axes, and everything under the part follows, skinned meshes included, so an app can turn a creature's head towards the camera while its walk clip plays, swing a rigid leg, or open a hinged lid by exactly as much as its own state says.

The host moves the part there over `seconds`, in a straight line from where it is now, on its own display clock. An app that works out a pose on each tick of its own clock and sends it with the tick's length is drawn smoothly between ticks. `Transform::IDENTITY` puts the part back. The op is new, so batches without it encode exactly as before.
