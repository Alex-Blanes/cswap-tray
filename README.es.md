# cswap-tray

*[Read this in English](README.md)*

Icono de bandeja para Windows que muestra qué cuenta de Claude Code está activa
y cuánta cuota le queda, y permite cambiarla de un clic.

Es una capa fina sobre [claude-swap](https://github.com/realiti4/claude-swap):
`cswap` sigue siendo el dueño de las credenciales, las cuotas y el cambio de
cuenta. Esta app solo lee `cswap list --json` y lanza `cswap switch`.
claude-swap trae un `menubar` para macOS; esto cubre el hueco en Windows.

## El icono

```
┌──────────────┐
│▞██████    ▓ ░│   punto rojo = una cuenta pide iniciar sesión
│ ██  ██    ▓ ░│   letra  = identidad de la cuenta, con su color
│ ██████    ▓ ▓│   barra izquierda = ventana de 5 h
│ ██        ▓ ▓│   barra derecha   = ventana de 7 d
│ ██        ▓ ▓│   se llenan de abajo arriba
└──────────────┘   verde <70 %, ámbar 70-90 %, rojo ≥90 %
```

- **Clic izquierdo**: rota a la siguiente cuenta. Con dos cuentas, alterna.
- **Clic derecho**: menú con cada cuenta y su uso (incluida la hora de reinicio
  de cada ventana), auto-cambio, precalentado, la leyenda del icono, las
  acciones y el comando de DeepSeek cuando está configurado.
- **Pasar el ratón**: resumen de todas las cuentas.

Cuando el *refresh token* de una cuenta muere, `cswap` deja de poder leer su
cuota y el auto-cambio deja de considerarla como destino. Antes eso era mudo:
ahora aparece el punto rojo en el icono estés en la cuenta que estés, la entrada
del menú pasa a `requiere iniciar sesión` y salta un aviso — al caducar y de
nuevo en cada arranque, porque arreglarlo (`/login` en Claude Code y luego
`cswap add`) solo puedes hacerlo tú y un ordenador que has apagado olvida que
pasó.

Nombrar un problema no es arreglarlo, y lo que se olvida son los comandos, así
que **al pulsar ese aviso se abre una consola que lleva el arreglo paso a
paso**. La propia entrada de la cuenta muerta en el menú abre la misma guía, que
es la vuelta cuando el aviso ya se ha esfumado. Empieza preguntándole a `cswap`
con qué cuenta está Claude Code *ahora mismo*: si ya iniciaste sesión y
simplemente no llegaste a lanzar `cswap add`, lo único que falta es guardar el
token y el paso del login se salta. Si no, activa antes la cuenta —para que la
credencial nueva caiga en su hueco y no encima de la de una cuenta viva—,
espera a que hagas el `/login`, la guarda y comprueba que la cuenta ha vuelto de
verdad.

No hace falta que salgas de Claude Code para cerrar el arreglo: la guía lanza su
`cswap add` cuando `claude` termina, pero la bandeja se adelanta. Desde que abres
la guía vigila la fecha del almacén de credenciales —solo la fecha, nunca el
contenido— y en cuanto ve que ha entrado un login guarda el token ella misma y te
avisa. La llamada de la guía se encuentra entonces el trabajo hecho y se limita a
confirmarlo.

Los porcentajes respetan la hora de reinicio. `cswap` solo refresca el uso
cuando algo llama a la API, así que al llegar al tope la lectura se queda
clavada en el 100 % mucho después de que la ventana se reinicie; cualquier
ventana cuya hora de reinicio ya pasó se lee como 0 %.

Todo se dibuja en tiempo de ejecución: sin imágenes ni fuentes externas.

## Requisitos

- Windows 10/11
- [claude-swap](https://github.com/realiti4/claude-swap) instalado y con al menos
  una cuenta añadida (`uv tool install claude-swap` y luego `cswap add`)

## Compilar

```powershell
cargo build --release
# target\release\cswap-tray.exe  (~0,5 MB, sin runtime)
```

## Arranque con Windows

```powershell
.\scripts\install-autostart.ps1            # instalar
.\scripts\install-autostart.ps1 -Uninstall # quitar
```

Windows 11 esconde los iconos nuevos en el desbordamiento (la flecha `^` de la
bandeja). Para fijarlo: *Configuración → Personalización → Barra de tareas →
Otros iconos de la bandeja del sistema* y activar `cswap-tray`.

## Cambio automático

Vives en tu cuenta preferida; cuando se agota, la app pasa sola a otra con
hueco; y vuelve a la preferida en cuanto se recupera.

```
       preferida al 90 %  ──────────────►  otra cuenta con hueco
       preferida < 80 %   ◄──────────────
```

Se enciende desde el submenú *Cambio automático*, que además explica la regla
ahí mismo. Los valores viven en `config.json`:

| Ajuste | Por defecto | Qué hace |
|---|---|---|
| `enabled` | `false` | Interruptor general |
| `preferred` | primera cuenta personal detectada | Dónde quieres estar por defecto |
| `switch_at_pct` | `90` | Se abandona la cuenta activa al llegar aquí |
| `return_below_pct` | `80` | Se vuelve a la preferida al bajar de aquí |
| `cooldown_seconds` | `300` | Espera mínima entre cambios |

Se mira la ventana que **primero** limite, sea la de 5 h o la de 7 d. Si están
todas agotadas no hace nada, y una cuenta cuyo uso no se puede leer nunca se
elige como destino.

El hueco entre `switch_at_pct` y `return_below_pct` es la histéresis: sin él,
una cuenta rondando el umbral provocaría un ida y vuelta continuo.

> **No lo combines con `cswap auto`.** Ese motor no tiene cuenta preferida: su
> estrategia `best` salta a la que más cuota tenga y se queda ahí. Con los dos
> en marcha, cada uno desharía los cambios del otro.

> **Un cambio te puede pillar a mitad de conversación.** Claude Code recoge las
> credenciales nuevas en el siguiente mensaje, así que esa sesión continuará
> contra la otra cuenta sin avisar. Es lo que se busca al activarlo, pero
> conviene saberlo: mezcla el gasto personal y el de trabajo.

## DeepSeek como último recurso

Opcional, apagado si no lo rellenas, y la única parte de la bandeja que puede
gastar dinero. DeepSeek sirve una API compatible con la de Anthropic, así que
Claude Code puede hablar con ella sin nada más — pero es una **sesión**, no una
tercera cuenta a la que cambiar.

```json
"deepseek": {
  "python": "C:/Python313/python.exe",
  "script": "C:/ruta/a/deepseek-key.py",
  "fallback": true
}
```

`script` apunta a un ayudante tuyo que imprime la clave de la API por stdout al
ejecutarlo como `<python> <script> get`, y sale con código distinto de cero si
no hay clave. Con `script` vacío la función no existe: ni entrada en el menú ni
recurso automático. Guarda la clave en el administrador de credenciales de
Windows o en otro almacén — nunca en un fichero que puedan leer la bandeja o el
repositorio.

- **El menú copia el comando.** *DeepSeek → Copiar comando* deja `claude-ds`
  (o `claude-ds -r`) en el portapapeles; pégalo en luvus o en una terminal del
  proyecto. No se lanza ninguna ventana: la sesión nace en el panel en el que
  estás trabajando.
- **`claude-ds` en cualquier terminal.** La bandeja instala `claude-ds.cmd` al
  lado de `claude.exe`, así que una sesión es `claude-ds` — los mismos
  argumentos que `claude`, sin ningún parámetro que recordar.
- **`-r` continúa una conversación de DeepSeek.** `claude-ds -r DeepSeek` abre
  el selector de sesiones de Claude Code con *DeepSeek* como término de
  búsqueda, así que la lista son las que este lanzador llamó así y no se retoma
  por accidente una conversación normal contra el endpoint de pago. El
  historial de una sesión con tool search puede tropezar con el 422 de abajo.
- **Por qué no es un interruptor.** Claude Code resuelve la credencial al
  arrancar, mientras que la URL base de los ajustes puede llegar a una sesión
  ya abierta. Parchear `~/.claude/settings.json` mueve por tanto la URL de una
  sesión viva a DeepSeek pero deja en su sitio el token OAuth de la
  suscripción: ese token es el que llega a api.deepseek.com —un 401—, y la URL
  se queda pegada al quitar el parche. Desde fuera del proceso no tiene
  arreglo; un fichero por sesión mantiene las dos mitades juntas.
- **Tool search va apagado.** Claude Code 2.1.286+ manda bloques
  `tool_definition` que DeepSeek rechaza con un 422 que su fallback no reconoce
  ([deepseek-ai/DeepSeek-V3#1701](https://github.com/deepseek-ai/DeepSeek-V3/issues/1701)).
  La sesión fija `ENABLE_TOOL_SEARCH=false`, así que todas las herramientas MCP
  se declaran de entrada: unos 50k tokens de contexto por sesión nueva, pagados
  por uso.
- **`fallback`** copia ese comando cuando el cambio automático ve todas las
  suscripciones agotadas. El portapapeles se sobrescribe una vez por sequía; no
  se factura nada hasta que lo pegues y mandes un mensaje.
- Contra DeepSeek no funcionan la caché de prompts, leer PDF, los conectores de
  claude.ai ni `/schedule`.

## Claude Code en el NAS

La bandeja lee y cambia las cuentas de los Claude Code que corren en el NAS, uno
por proyecto (un contenedor por proyecto). No hay ningún agente al otro lado:
cada contenedor ya lleva su `cswap`, y la bandeja lo maneja por SSH con
`docker exec`.

```json
"nas": {
  "ssh": "nas",              // alias de ~/.ssh/config; vacío apaga la sección
  "sudo": true,              // docker necesita sudo en el NAS
  "filter": "claude-code",   // nombre o imagen que deben contener
  "refresh_seconds": 60
}
```

Los contenedores se descubren solos: el que se llame, o use una imagen, que
contenga `filter` aparece en el submenú `NAS` en la siguiente lectura. Dentro de
cada uno:

- **Cuentas**: clic para cambiar. El cambio manual pausa `cswap-preferida` en
  ese contenedor; si no, volvería a la preferida en menos de un minuto.
- **Auto**: pausa o reanuda ese demonio.
- **Preferida**: a qué cuenta vuelve el automático. La primera vez instala la
  versión del script que lee la preferencia de `~/.config/cswap-preferida.json`
  (deja la anterior como `.bak`); después basta con escribir ese fichero.
- **DeepSeek (API)**: aprovisiona el lanzador dentro del contenedor y abre una
  sesión de pago en una terminal. La clave viaja por stdin, nunca en un
  argumento, y queda en un fichero 600 leído por el `apiKeyHelper`.

La sesión que ya esté abierta coge la cuenta nueva en el siguiente mensaje:
`cswap switch` escribe `.credentials.json` y Claude Code suelta el token
cacheado.

## Precalentar la cuenta de reserva

La ventana de 5 h de una cuenta **la abre su primer mensaje** — no el login ni
el cambio de credencial. Si trabajas siempre en la misma, la otra tiene el
contador parado, y el día que la necesites su ventana empezará justo entonces.

Precalentar no reserva cuota: **adelanta el reloj**.

```
sin precalentar   09:00 ─────────── 13:00 estrenas reserva ──────── 18:00 se renueva
con precalentar   09:00 mensaje ────────────────── 14:00 se renueva
```

Cuando la app detecta que estás trabajando y la reserva tiene el contador
parado, avisa una vez con una notificación de Windows. En el submenú
*Precalentar reserva* hay dos formas de arrancarlo:

- **Pasar mi próximo mensaje por X** — cambia a esa cuenta y te devuelve solo en
  cuanto detecta que le ha entrado el mensaje. Aprovecha un mensaje que ibas a
  mandar igualmente; no genera consumo extra. Si en `arm_timeout_minutes` no
  llega ninguno, deshace el cambio y te avisa.
- **Ejecutar mi tarea de precalentado en X** — lanza un prompt tuyo en esa
  cuenta con `cswap run`, que aplica la credencial **solo a ese proceso**: la
  cuenta que estás usando no se toca. La respuesta se guarda en
  `last-prewarm-output.md` y se abre desde el menú.

A propósito no viene ningún prompt de fábrica. `prewarm.task` empieza vacío y la
entrada del menú está deshabilitada hasta que lo definas, porque la idea es
mandarle a esa cuenta **trabajo que de verdad quieres y cuya respuesta lees**.
Un prompt enlatado que nadie lee sería una petición cuyo único fin es el reloj,
y eso es otra cosa — ver [Uso legítimo](#uso-legítimo).

| Ajuste | Por defecto | Qué hace |
|---|---|---|
| `notify` | `true` | Avisar cuando la reserva está fría |
| `arm_timeout_minutes` | `15` | Espera antes de deshacer un precalentado armado |
| `task` | vacío | Tu propio prompt para ejecutar en la reserva |

La detección del mensaje es indirecta: se reconoce porque a esa cuenta le
aparece ventana de 5 h o le sube el consumo. Como el sondeo es cada 30 s, la
vuelta a tu cuenta tarda hasta un minuto — y en ese hueco cualquier otro
mensaje también irá por la reserva. Con varias sesiones de Claude abiertas a la
vez, tenlo en cuenta.

Para comprobar que los avisos de Windows llegan:

```powershell
.\target\release\cswap-tray.exe --test-toast
```

Y para ensayar el camino completo del re-login —el aviso, su botón y la consola
que abre el clic— sin esperar a que caduque un token:

```powershell
.\target\release\cswap-tray.exe --test-relogin
```

Apunta a la cuenta en la que ya estás, así que todo lo que lanza la guía es un
refresco inofensivo de una cuenta viva. El clic se atiende en el propio proceso,
por eso el ensayo se queda vivo 90 s; la bandeja, que siempre lo está, no
necesita nada de eso.

## Apariencia e idioma

Se autogenera en `%APPDATA%\cswap-tray\config.json` la primera vez, deduciendo
qué cuenta es personal por el dominio del correo:

```json
{
  "language": "auto",
  "refresh_seconds": 30,
  "accounts": {
    "tu@hotmail.com":  { "name": "Personal",  "letter": "P", "color": "#3B82F6" },
    "tu@empresa.com":  { "name": "Empresa",   "letter": "W", "color": "#F59E0B" }
  }
}
```

`letter` admite A-Z y 0-9. `language` acepta `"auto"` (sigue al idioma de
Windows), `"en"` o `"es"`: toda la interfaz está traducida. Los cambios se
aplican al reiniciar la app. Las
cuentas se siguen añadiendo y quitando con `cswap`, no aquí; las nuevas
aparecen solas en el fichero al detectarlas.

`refresh_seconds` es solo la cadencia de lectura: cswap cachea el uso en disco,
así que sondear no dispara llamadas a la API.

## Cómo está montado

| Fichero | Responsabilidad |
|---|---|
| `src/cswap.rs` | Invocar el CLI y deserializar `list --json` (schemaVersion 1) |
| `src/auto.rs` | Política de cambio automático: función pura sobre la instantánea |
| `src/prewarm.rs` | Detección de reserva fría y de que el mensaje ha aterrizado |
| `src/deepseek.rs` | Ajustes de DeepSeek por sesión y el lanzador `claude-ds` |
| `src/nas.rs` | Contenedores del NAS por SSH: cuentas, preferida y DeepSeek |
| `src/clipboard.rs` | Escribir el comando copiado en el portapapeles de Windows |
| `src/icon.rs` | Dibujar el icono RGBA: fuente 5×7 propia + barras |
| `src/i18n.rs` | Cadenas de la interfaz en inglés y castellano |
| `src/config.rs` | Apariencia por cuenta y heurística personal/trabajo |
| `src/main.rs` | Icono de bandeja, menú, bucle de mensajes Win32 y worker |

El sondeo y los cambios de cuenta corren en un hilo aparte para que el CLI (que
es Python y tarda ~1 s en arrancar) no bloquee la interfaz.

```powershell
cargo test                                        # 51 tests
cargo test preview -- --ignored --nocapture       # previsualizar el icono en ASCII
```

## Limitaciones conocidas

- Solo Windows: usa el bucle de mensajes Win32 directamente.
- El tamaño del icono se decide al arrancar (`SM_CXSMICON`); si cambias la escala
  de pantalla, reinicia la app.
- La tooltip de Windows corta a 127 caracteres, así que con muchas cuentas el
  resumen se trunca. El menú sí las muestra todas.

## Uso legítimo

Esta herramienta da por hecho que tienes más de una suscripción de Claude
propia — una personal y otra del trabajo, por ejemplo — y solo se mueve entre
cuentas que ya son tuyas. No comparte nada con terceros ni envía nada que no
hayas pedido.

De las Condiciones de Consumo de Anthropic, cuya sección 3 restringe acceder al
servicio "through automated or non-human means", salen dos decisiones de diseño:

- **No se manda nada sin que lo pidas.** Cambiar de cuenta, armar tu próximo
  mensaje y ejecutar tu tarea son cosas que pulsas tú. La app nunca envía un
  mensaje por su cuenta.
- **No incluye ningún prompt sintético.** `prewarm.task` lo escribes tú, y la
  respuesta se guarda para que la leas. Un ping de fábrica cuyo único fin fuera
  arrancar el reloj de un límite sería automatización dirigida a los límites en
  sí, que es justo de lo que habla esa cláusula.

Si dudas de si tu uso encaja con tu plan, lee las condiciones tú mismo: esto no
es asesoramiento legal.

## Licencia

MIT — ver [LICENSE](LICENSE).
