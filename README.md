# Jner DAW

DAW de escritorio nativo en Rust. Implementación actual: **M10 — Exportación WAV**, con render offline estéreo, frecuencia y duración configurables, además de grabación, automatización, efectos, sintetizador, piano roll, drum machine, mixer, timeline y proyectos. La captura completa con micrófono sigue pendiente de validación.

## Ejecutar

Requisitos iniciales: Windows 10/11, Rust 1.97 o posterior y las herramientas de C++ de Visual Studio para el target `x86_64-pc-windows-msvc`. Entorno comprobado: Rust/Cargo 1.97.1. Es necesario disponer de una salida de audio activa.

```sh
cargo run --locked
```

La ventana abre la salida predeterminada en silencio, con un kit interno de kick, snare, hi-hat y clap. Pulsa **Play** para escuchar el patrón de ejemplo; **Stop** vuelve el reloj a cero. El master inicial es 8%, con un máximo de 25%. El nivel final también depende del volumen del sistema.

La interfaz usa navegación lateral, tarjetas oscuras y transporte fijo abajo. Selecciona el editor desde la izquierda; **Audio y captura** reúne los dispositivos y la grabación, y **Exportar audio** abre la exportación WAV. Los efectos permanecen debajo del editor.

- Activa o desactiva cualquiera de los **4 × 16 pasos**, incluso durante la reproducción. El borde blanco indica el paso actual.
- Ajusta el **BPM entre 30 y 300**, con arrastre o doble clic para escribir/pegar un valor. Los cambios de tempo mantienen el progreso dentro del paso actual.
- **Limpiar** vacía el patrón; **Ejemplo** recupera el beat inicial sin reemplazar tus samples.
- **Cargar WAV** sustituye el sonido de una fila. Admite WAV mono/estéreo de hasta 10 segundos, de 8 a 192 kHz, PCM 8/16/24/32 bits o float 32 bits. El estéreo se convierte a mono y se remuestrea a la salida antes de reproducirse.
- Al completar una carga válida se detiene el transport: pulsa Play para escuchar el nuevo sample. Un WAV no válido muestra un error y conserva el sonido anterior.
- La pestaña **Mixer** ofrece volumen de 0 a 200%, panorama L/Centro/R, Mute y Solo para cada pista. El 100% es ganancia unitaria. Puedes combinar varias pistas en Solo; Mute tiene prioridad.
- El mixer muestra picos por pista y un master estéreo L/R. **Reiniciar mixer** restaura volumen al 100%, panorama al centro y desactiva Mute/Solo.
- Los ajustes del mixer se conservan al detener/reiniciar, cambiar o reconectar la salida y cargar un WAV, durante la sesión actual. Cambiar entre Secuenciador y Mixer mantiene la reproducción.

## Timeline

Selecciona **Timeline** para reproducir clips de audio en cuatro pistas. Cambiar entre Secuenciador y Timeline detiene y reinicia el transport; Mixer conserva el modo que estaba activo. La línea superior indica **Modo: Patrón** o **Modo: Timeline**.

- **Crear ejemplo** añade un arreglo de 15 clips si el timeline está vacío. Reutiliza el kit actual del patrón como fuentes independientes.
- Elige una pista y el beat de inserción, y usa **Sonido de pista** o **Importar WAV**. También puedes seleccionar el punto de inserción haciendo clic en la regla. Este cursor no cambia el punto de inicio de Play.
- Arrastra el cuerpo del clip para moverlo en tiempo o cambiarlo de pista. Arrastra sus bordes para recortar o recuperar audio hasta los límites de la fuente.
- **Snap 1/16** ajusta posiciones y bordes a semicorcheas; desactívalo para editar libremente. Los límites de la fuente tienen prioridad sobre el snap.
- **Zoom** modifica la escala; la barra horizontal permite desplazarse. La regla numera compases de 4/4 y el playhead muestra compás y beat.
- Haz clic en un clip para seleccionarlo. El detalle inferior muestra el tramo de fuente en segundos. **Quitar clip** lo elimina del arreglo, sin modificar el archivo original.
- Añadir, quitar o arrastrar clips detiene y reinicia la reproducción. Play siempre comienza al principio. Los clips se reproducen una vez; después el reloj continúa en silencio hasta Stop. No hay seek, loop de arreglo ni seguimiento automático de la vista en esta entrega.

El timeline conserva sus propias referencias de audio: reemplazar un sonido del patrón no cambia clips existentes. La sesión admite hasta 32 clips y 16 fuentes de hasta 10 segundos cada una, con inicios dentro de 256 beats. Al quitar clips se liberan las fuentes que ya no se usan. Cambiar el BPM modifica las posiciones musicales futuras; el audio mantiene su velocidad y pitch, sin time-stretch.

## Transport y salida

- **Espacio:** Play/Stop. **Esc:** Stop.
- Cambiar la salida, **Actualizar** o **Reconectar** detiene y reinicia el transport.
- **Actualizar** enumera de nuevo y selecciona la salida predeterminada. **Reconectar** reabre la selección actual.
- La opción marcada como predeterminada sigue la salida del sistema. La entrada con el mismo nombre sin esa marca selecciona el dispositivo físico.
- Los errores de apertura aparecen en la ventana. Ante una desconexión, selecciona otra salida o utiliza Actualizar/Reconectar.

Las frecuencias, canales y formato los decide la configuración predeterminada del dispositivo; todavía no se configuran manualmente. El buffer lo asigna el sistema. En **Diagnóstico de audio**, el tamaño del último callback no representa la latencia de extremo a extremo. La interfaz admite desplazamiento vertical en ventanas pequeñas.

## Piano roll

La pestaña **Piano roll** permite editar notas internas en una rejilla de 16 compases 4/4, hasta 256 notas repartidas entre cuatro pistas. **Play reproduce las notas con el sintetizador interno**, a través del mixer y el master. No envía MIDI a dispositivos externos ni reproduce los samples del patrón.

- Haz **doble clic** en un espacio vacío para crear una nota. Elige antes pista, velocity (1–127) y duración para las notas nuevas. **Crear ejemplo** añade cuatro notas cuando la secuencia está vacía.
- Selecciona una nota y arrastra su cuerpo para cambiar inicio y pitch; arrastra su **borde derecho** para cambiar la duración. **Snap 1/16** ajusta tiempos a semicorcheas; sin snap la resolución es de un tick (960 por beat). Los límites de la rejilla prevalecen sobre el snap.
- El inspector permite editar pitch MIDI (0–127), pista, velocity, inicio y duración. La intensidad del color refleja la velocity. **Borrar nota** o **Supr** elimina la selección.
- Usa **Zoom**, las barras de desplazamiento y **Centrar C4** para navegar. C4 corresponde al pitch MIDI 60.
- Cambiar entre Patrón, Timeline y Piano roll detiene y reinicia el transport. Mixer conserva el modo activo. Play comienza en cero y recorre las notas una vez; el reloj sigue avanzando tras la última. Los contadores muestran eventos On/Off y notas activas; Stop reinicia los contadores y elimina la actividad.
- Editar notas detiene la reproducción y activa el asterisco de cambios sin guardar. El patrón y los clips de audio se conservan por separado.

## Sintetizador

En Piano roll, elige una pista y despliega **Sintetizador · Pista…** para ajustar su sonido. **Crear ejemplo** añade cuatro notas si la secuencia está vacía; pulsa Play para escucharlas.

- Cada pista tiene su propia onda **Sine / Square / Triangle / Saw**, envolvente **Attack / Decay / Sustain / Release** y filtro pasa bajos.
- Attack, Decay y Release se expresan en milisegundos (0–5000); Sustain fija el nivel mantenido (0–1). La velocity escala el volumen de cada nota. El filtro permite un corte nominal de 20–20 000 Hz, acotado por la frecuencia de salida.
- Los cambios de sonido se aplican a las **notas que comiencen después**. Las notas y colas que ya suenan mantienen su sonido anterior. Stop/Play vuelve a reproducir con todos los ajustes nuevos. **Restablecer sonido** restaura únicamente el instrumento de la pista elegida.
- Las cuatro pistas comparten **16 voces**. Si se supera el límite, se sustituye primero una voz en release y luego la más antigua, con una transición corta. Las notas del proyecto se conservan aunque alguna no disponga de voz.
- Gain, panorama, Mute, Solo y master afectan al synth mediante el mixer existente. En modo Piano roll, Mixer muestra las pistas y sus ondas. Tras el último Note Off se escucha la cola de Release; **Stop** apaga la salida mediante la rampa corta del transport y limpia las voces.

El instrumento no reproduce al abrir el proyecto. Las tablas de onda y los coeficientes se preparan fuera del callback. No hay resonancia, modulación, teclado MIDI externo ni mezcla simultánea con el patrón/timeline en esta entrega. A sample rates bajos, las notas cuya frecuencia fundamental alcanza el 45% del sample rate quedan en silencio.

## Delay y distorsión

Despliega **Efectos master · Distorsión → Delay**, debajo del transport. Son efectos del bus estéreo: actúan sobre la suma del mixer en cualquiera de los modos, antes del volumen master. Cada efecto tiene su propio interruptor; ambos empiezan desactivados.

- **Distorsión:** soft clipping con Drive de 1 a 20 y mezcla seco/procesado de 0 a 1. Aumentar Drive cambia la saturación; Mezcla 0 conserva la señal seca y 1 usa la procesada.
- **Delay:** tiempo de 10 a 1000 ms, Feedback de 0 a 0.85 y mezcla de 0 a 1. Los canales L/R tienen ecos independientes, sin intercambio entre ellos. No está sincronizado al BPM.
- Puedes activar, desactivar y mover parámetros mientras suena. Las rampas de 20 ms mantienen el stream y el reloj en marcha. Cambiar el tiempo del delay puede producir un deslizamiento audible de pitch mientras se mueve la lectura.
- El bypass del delay retira gradualmente su envío y salida; la cola interna sigue decayendo y puede volver a oírse si lo reactivas pronto. Al terminar clips o notas, los ecos siguen mientras Play permanezca activo. Mute/Solo afectan a la entrada del bus: los ecos ya almacenados pueden continuar.
- **Stop** desvanece la salida con la rampa corta del transport y limpia el eco. Play vuelve a empezar sin audio del recorrido anterior. Cambiar modo, reconectar, abrir proyecto o editar recursos también reconstruye la salida detenida.
- **Restablecer efectos** recupera los valores iniciales y desactiva ambos. Los ajustes se guardan con el proyecto y activan el asterisco de cambios sin guardar.

La memoria de un segundo de delay se reserva antes de abrir CPAL. Los cambios en vivo envían solo parámetros preparados; no sustituyen buffers dentro del callback. EQ, reverb, compresor, efectos por pista y reordenación de la cadena quedan fuera de esta entrega.

## Automatización

La pestaña **Automatización** conserva el modo de reproducción que estuviera activo —Patrón, Timeline o Piano roll— y permite editar sus curvas. Hay nueve destinos: volumen y panorama de cada una de las cuatro pistas, y mezcla del delay.

- Elige un destino y haz **doble clic** en la rejilla para añadir puntos. Arrastra un punto para cambiar tiempo y valor; selecciónalo para editar beat/valor en el inspector. **Borrar punto** o **Supr** elimina la selección. **Añadir en beat 1** usa el valor manual de base, si no existe ya un punto allí.
- **Crear ejemplo** genera una curva de tres puntos para el destino elegido si está vacío. **Leer curva** activa o desactiva ese carril conservando sus puntos. Un carril vacío no altera el audio.
- **Snap 1/16** ajusta el tiempo a semicorcheas; sin snap la edición tiene resolución de un tick. Hay zoom y desplazamiento horizontal. Cada carril admite 32 puntos dentro de 16 compases 4/4 (64 beats, con el borde final en beat 65). Dos puntos de un mismo carril no pueden compartir tick.
- Volumen usa 0–2: 0 es silencio, 1 equivale a 100% y 2 a 200%. Panorama usa -1/0/1 para izquierda/centro/derecha. La mezcla del delay usa 0–1 para seco/eco; requiere activar **Delay** en Efectos master y respeta su bypass.
- Entre puntos se interpola linealmente; antes del primero se mantiene su valor y después del último se mantiene el final. Las curvas recorren el tiempo global una vez: el patrón de batería puede seguir en loop, pero las curvas no reinician con cada loop.
- Durante Play, cada carril activo sustituye el valor manual de su parámetro. Los controles manuales siguen mostrando/guardando la base; sus cambios se escuchan cuando ese parámetro no está automatizado. Mute/Solo y master se conservan. **Stop** restaura el control manual y Play reinicia las curvas desde cero.
- Editar puntos o activar/desactivar lectura detiene y prepara de nuevo la reproducción. Cambiar de vista hacia Automatización o Mixer conserva la reproducción. Los valores en pantalla son telemetría aproximada; el audio evalúa las curvas desde el reloj de muestras y sigue los cambios de BPM.

Las curvas, puntos y estados de lectura se guardan con el proyecto. No hay grabación de movimientos, modos touch/latch, curvas Bézier, loop de automatización, undo/redo ni automatización de otros parámetros en esta entrega.

## Grabar una entrada

Despliega **Grabación de entrada · toma mono de hasta 10 s**, selecciona una entrada y una pista de destino, y pulsa **Grabar desde inicio**. La entrada se abre únicamente al pulsar ese botón. **Actualizar entradas** solo vuelve a enumerar dispositivos.

- La grabación reproduce el **timeline desde beat 1**, con sus clips actuales como acompañamiento. No reproduce simultáneamente el patrón ni el piano roll. La toma comienza en ese mismo origen y se incorpora como un clip independiente en la pista elegida.
- **Detener toma**, Espacio o Escape finalizan la captura. A los **10 segundos** termina automáticamente. **Cancelar toma** descarta únicamente la captura en curso y conserva los clips existentes.
- Se utiliza la configuración predeterminada de la entrada: 8–192 kHz y 1–8 canales, convertidos por promedio a una toma mono. El audio se conserva seco; mixer, efectos y automatización actúan al reproducir el clip, sin quedar impresos en la fuente.
- Mientras se graba, los controles de edición, dispositivos y archivos quedan bloqueados. Para cerrar la ventana, primero termina o cancela la toma. Después de grabar, pulsa Play para escuchar el timeline y guarda el proyecto para conservarla.
- Las tomas quedan incluidas en el `.jner` como las demás fuentes. No se crea un WAV externo ni se cambia la versión 5 del formato.

No hay monitorización de entrada, selección de canales individuales, punch-in, seek de grabación ni captura multipista. La alineación usa timestamps del dispositivo y contadores de muestras; si el primer timestamp de entrada no es utilizable, se estima el origen restando la duración del paquete a su llegada y se avisa al completar la toma. No se ha calibrado la latencia física ni la deriva entre dispositivos. Se inspeccionó el panel real y se reprodujo un fallo de timestamps; la toma completa con la alternativa nueva sigue pendiente.
## Exportar audio

Despliega **Exportar WAV · estéreo PCM 16 bits**. Se exporta el modo de reproducción activo: Patrón, Timeline o Piano roll. Mixer y Automatización conservan ese modo. El archivo empieza en beat 1 y utiliza el BPM, las fuentes, el synth, mixer, master, efectos y curvas actuales, incluidos los cambios que aún no has guardado.

- Elige **44 100, 48 000, 96 000 o 192 000 Hz** y la duración en **beats (1–512)**. Se muestra su equivalente en segundos. En 4/4, cuatro beats equivalen a un compás.
- **Ajustar al contenido** propone un compás para el patrón, el final de los clips redondeado al beat siguiente para el timeline, o el final de las notas incluyendo Release para el piano roll. Cambiar de modo propone de nuevo esa duración.
- **Exportar WAV…** abre el diálogo de destino. El render detiene la reproducción y trabaja en segundo plano sin abrir una entrada ni necesitar salida de audio. Muestra progreso y permite **Cancelar exportación**. Al terminar, la reproducción queda detenida.
- El patrón repite durante todo el intervalo. Timeline y piano roll conservan su recorrido de una pasada. Se corta exactamente al final elegido: aumenta la duración para incluir silencio final y colas de efectos. No se detecta automáticamente el final del delay ni se añade un fade al corte.
- El WAV usa dos canales PCM16, sin normalización ni dither, con el master actual (inicialmente 8%). Guardar un WAV no guarda el `.jner` ni elimina su asterisco de cambios pendientes.
- La escritura se realiza en un temporal del directorio de destino; solo se reemplaza el archivo final al completar el render. Cancelar o fallar conserva el destino anterior. La edición y las operaciones de proyecto quedan bloqueadas mientras trabaja; cancela o espera antes de cerrar.

Se mantiene la separación entre los tres modos de reproducción: no se suman patrón, timeline y piano roll en una sola mezcla. MP3, stems, RF64, normalización y empaquetado de instaladores quedan fuera de esta entrega.

## Guardar y abrir proyectos

La barra superior ofrece **Nuevo**, **Abrir**, **Guardar** y **Guardar como**. Los archivos `.jner` conservan BPM, patrón, master, volumen/panorama/Mute/Solo de las cuatro pistas, clips con sus recortes, notas con pitch/velocity/duración y todas sus fuentes de audio. Los samples quedan incluidos: el proyecto puede abrirse después de mover o retirar los WAV originales.

- **Ctrl+N / Ctrl+O / Ctrl+S / Ctrl+Shift+S:** Nuevo / Abrir / Guardar / Guardar como.
- El asterisco junto al nombre indica cambios sin guardar. Nuevo, Abrir y cerrar la ventana ofrecen **Guardar y continuar**, **Descartar** o **Cancelar** cuando hay cambios.
- Un guardado fallido conserva el archivo anterior y los cambios de la sesión. Un archivo inválido al abrir conserva la sesión actual.
- Abrir un proyecto o crear uno nuevo detiene y reinicia el transport. **Nuevo** empieza con patrón, timeline y notas vacíos, sin curvas, a 120 BPM y con el kit interno; el arranque sin archivo mantiene el patrón de demostración.
- Se conserva también la vista, el modo Patrón/Timeline/Piano roll y los ajustes de cursor de inserción, pista de inserción, zoom y snap del timeline. Los cambios de estos ajustes o de las vistas Mixer/Automatización se incluyen al guardar, aunque por sí solos no activan el asterisco. Los ajustes de navegación/selección del piano roll y la automatización se reinician al abrir.
- La escritura y lectura se realizan en un worker; durante la operación se desactiva la edición. Los diálogos nativos son modales. El guardado no detiene el audio; abrir prepara de nuevo la salida seleccionada y deja la reproducción detenida.

También puedes abrir un archivo al iniciar:

```sh
cargo run --locked -- --open "mi-proyecto.jner"
```

El formato actual es JSON **versión 5**, con PCM mono float32 incluido en base64, sin compresión, synth, efectos y carriles de automatización. Se pueden abrir archivos v1–v4 de M4–M7: reciben curvas vacías y conservan los datos soportados por su versión. Los que no tengan synth reciben el instrumento predeterminado; los que no tengan efectos los reciben desactivados. Las notas de M5 pasan a ser audibles al pulsar Play. Guardar migra a v5, incompatible con ejecutables anteriores. Límite de archivo: 256 MiB. No hay autoguardado ni recuperación tras un cierre inesperado. La selección de dispositivo y el estado de reproducción no se guardan.

La exportación WAV de M10 es independiente del guardado de proyectos v5. No cambia su formato.

El panorama usa potencia constante: al centro cada canal recibe aproximadamente el 70.7% de la señal de la pista antes del master. Por eso el nivel por canal al centro es menor que en M1. En salidas mono se promedian L/R; en dispositivos de más canales se usan los dos primeros y se silencian los restantes.

## Diagnóstico por consola

```sh
cargo run --locked -- --list-devices
cargo run --locked -- --smoke-test
cargo run --locked -- --smoke-test 3
cargo run --locked -- --smoke-beat
cargo run --locked -- --smoke-mixer
cargo run --locked -- --smoke-timeline
cargo run --locked -- --smoke-project
cargo run --locked -- --smoke-midi
cargo run --locked -- --smoke-synth
cargo run --locked -- --smoke-effects
cargo run --locked -- --smoke-automation
cargo run --locked -- --list-inputs
cargo run --locked -- --smoke-recording
cargo run --locked -- --smoke-export
cargo run --locked -- --export entrada.jner salida.wav 48000 16
```

`--smoke-test` utiliza la salida predeterminada o el índice de la lista actual. Reproduce unos dos segundos de tono y comprueba silencio inicial, avance del reloj, Stop, silencio posterior y reinicio. Termina con un error si no llegan callbacks, falla el stream o no se cumplen esas condiciones. Esta comprobación requiere hardware y emite sonido; no sustituye la escucha manual.

`--smoke-beat [índice]` comprueba dos loops del kit interno, cambios de tempo/master/patrón mientras suena, silencio al limpiar el patrón y Stop/reinicio. El tono de 440 Hz de M0 permanece disponible por `--smoke-test`.

`--smoke-mixer [índice]` requiere una salida de al menos dos canales. Comprueba panoramas extremos, volumen cero, Solo múltiple, Mute sobre Solo y conservación de ajustes tras Stop/Play, sin detener el reloj al editar. Los picos verificados pertenecen al buffer generado, no a una grabación de la salida física.

`--smoke-timeline [índice]` reproduce tres clips del kit interno, uno movido y recortado con snap. Comprueba el reloj musical, el silencio después de terminar los clips, cambio de BPM y Stop/Play.

`--smoke-project` guarda, abre, guarda una segunda copia y vuelve a abrir un proyecto temporal. Compara parámetros y, antes/después, 96 000 frames estéreo del timeline y otros 96 000 del synth, con efectos y automatización. No usa hardware ni emite sonido.

`--smoke-midi [índice]` requiere un dispositivo activo, pero no genera sonido. Comprueba tres Note On/Off, cambio de BPM, silencio del buffer y Stop/Play sin actividad residual.

`--smoke-synth [índice]` reproduce un acorde de cuatro voces con ondas diferentes, comprueba ADSR y silencio al terminar las colas, cambio de sonido, Stop y reinicio. Requiere hardware y emite sonido al master inicial de 8%; no sustituye la escucha humana.

`--smoke-effects [índice]` reproduce el beat y cambia drive y tiempo de delay en vivo, comprobando continuidad de reloj y callbacks, cola después de vaciar el patrón, bypass y ausencia de eco antiguo tras Stop/Play. Emite sonido al master inicial de 8%; no sustituye la escucha humana.

`--smoke-automation [índice]` reproduce el beat con tres curvas —volumen, panorama y mezcla del delay—, verifica sus valores intermedios/finales, cambia BPM y comprueba Stop/reinicio. Emite sonido a master 8%; no sustituye la escucha humana.

`--list-inputs` enumera entradas sin abrirlas. `--smoke-recording` pasa audio sintético por la captura, crea un clip, guarda/reabre y compara 24 000 frames estéreo remuestreados a 48 kHz. No abre micrófonos, usa una ventana ni emite sonido; no valida la captura física.

`--export entrada.jner salida.wav sample_rate beats` renderiza el modo guardado en el proyecto sin hardware. Acepta 8–192 kHz. `--smoke-export [salida.wav]` comprueba un ejemplo estéreo de dos segundos a 48 kHz y lee el WAV completo; sin destino explícito utiliza un directorio temporal. Ninguno emite sonido.

## Estructura

- `src/engine.rs`: transport, mezcla de voces y tono de diagnóstico, independientes del dispositivo y de egui.
- `src/mixer.rs`: gain, panorama estéreo, Mute/Solo y rampas de coeficientes.
- `src/sequencer.rs`: patrón, BPM y scheduler basado en muestras.
- `src/midi.rs`: notas internas, edición y programación acotada de eventos Note On/Off.
- `src/synth.rs`: osciladores por tablas, ADSR, filtro y 16 voces preasignadas.
- `src/effects.rs`: soft clipping, delay estéreo, parámetros preparados y rampas en vivo.
- `src/automation.rs`: puntos/carriles validados, interpolación y player acotado de curvas.
- `src/timeline.rs`: modelo de clips, edición no destructiva, reloj musical y reproducción acotada.
- `src/samples.rs`: kit interno, decodificación WAV y preparación de samples.
- `src/audio.rs`: descubrimiento de salidas, stream CPAL, cola acotada y telemetría.
- `src/recording.rs`: entradas CPAL, alineación de captura, ring buffer acotado y ensamblado de tomas fuera del callback.
- `src/export.rs`: render offline con el engine, conversión PCM16, progreso, cancelación y publicación del WAV.
- `src/ui.rs`: ventana nativa y comandos del usuario.
- `src/ui/timeline_view.rs`: regla, playhead, zoom y edición de clips.
- `src/ui/piano_view.rs`: rejilla de notas, selección, arrastres e inspector de velocity/pitch/duración.
- `src/ui/synth_view.rs`: controles del instrumento de cada pista y envío de parámetros preparados.
- `src/ui/effects_view.rs`: controles de los efectos del bus y envío de cambios sin reabrir el stream.
- `src/ui/automation_view.rs`: rejilla de puntos, lectura por carril, snap, zoom e inspector.
- `src/ui/recording_view.rs`: selección de entrada/pista, grabación, parada/cancelación y creación del clip.
- `src/ui/export_view.rs`: frecuencia/duración, selección del destino y seguimiento del worker de exportación.
- `src/project.rs`: formato versionado, validación, fuentes incluidas y reemplazo del archivo mediante un temporal.
- `src/ui/project_actions.rs`: operaciones en segundo plano, atajos y protección de cambios sin guardar.
- `src/main.rs`: arranque y diagnóstico por consola.

Dependencias directas fijadas: [eframe/egui 0.37.0](https://docs.rs/eframe/0.37.0/eframe/), [CPAL 0.18.2](https://docs.rs/cpal/0.18.2/cpal/), [rtrb 0.4.1](https://docs.rs/rtrb/0.4.1/rtrb/), [hound 3.5.1](https://docs.rs/hound/3.5.1/hound/), [rubato 5.0.1](https://docs.rs/rubato/5.0.1/rubato/), [rfd 0.17.2](https://docs.rs/rfd/0.17.2/rfd/), [serde 1.0.229](https://docs.rs/serde/1.0.229/serde/), [serde_json 1.0.154](https://docs.rs/serde_json/1.0.154/serde_json/), [base64 0.23.1](https://docs.rs/base64/0.23.1/base64/) y [tempfile 3.27.0](https://docs.rs/tempfile/3.27.0/tempfile/). `Cargo.lock` fija la resolución completa.

## Verificar

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

La [CI de GitHub Actions](.github/workflows/ci.yml) ejecuta esos cuatro comandos en Windows y añade `--smoke-project` y `--smoke-export`, sin dispositivo. Se inicia con cada push, pull request o desde **Actions → CI → Run workflow** cuando el repositorio esté publicado en GitHub. Los diagnósticos que requieren salida de audio y la escucha humana siguen siendo validaciones locales. Este flujo practica integración continua; aún no publica instaladores ni versiones.

Las 102 pruebas unitarias no necesitan hardware: verifican tono, relojes, reinicio, límites de buffers, canales, cola llena, timing de pasos/clips/notas, edición, lectura de WAV, resampling, solapamiento, master, mixer, ondas, ADSR, filtro, velocity, voces, efectos, curvas, cambios de BPM, compatibilidad de proyectos, conservación de fuentes y recuperación ante errores. Incluyen captura sintética, estimación de timestamp, alineación/parada, persistencia del clip, navegación lateral a dos tamaños y exportación WAV con cancelación, conservación del destino y silencio inicial de pistas en Mute. Después de descargar dependencias, puede añadirse `--offline` a los comandos de Cargo.

Contexto de producto: [roadmap en Notion](https://app.notion.com/p/40a69ed855c88151bdc4f97f989de7bd) y [Features y tareas](https://app.notion.com/p/3994e890ad374a8a80ad27d7444d0107).
