//! UI strings in English and Spanish.
//!
//! The language is picked from the Windows UI language unless `language` in
//! `config.json` forces one (`"en"` / `"es"`). Everything the user can read —
//! menu entries, the in-menu help text and the toasts — goes through here, so
//! adding a language means adding one arm per string and nothing else.

use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Es,
}

/// Primary language identifier for Spanish (LANG_SPANISH).
const LANG_SPANISH: u16 = 0x0A;

impl Lang {
    /// `setting` comes from the config: `"auto"`, `"en"` or `"es"`.
    pub fn resolve(setting: &str) -> Lang {
        match setting.trim().to_ascii_lowercase().as_str() {
            "en" => Lang::En,
            "es" => Lang::Es,
            _ => Lang::from_langid(unsafe { GetUserDefaultUILanguage() }),
        }
    }

    /// The low 10 bits of a Windows LANGID hold the primary language.
    pub fn from_langid(langid: u16) -> Lang {
        if langid & 0x3FF == LANG_SPANISH {
            Lang::Es
        } else {
            Lang::En
        }
    }
}

/// Picks the arm for the current language.
macro_rules! s {
    ($self:ident, $en:expr, $es:expr) => {
        match $self {
            Lang::En => $en,
            Lang::Es => $es,
        }
    };
}

impl Lang {
    // --- tray and root menu ------------------------------------------------
    pub fn loading(self) -> &'static str {
        s!(self, "cswap-tray — loading…", "cswap-tray — cargando…")
    }
    pub fn no_data(self) -> &'static str {
        s!(self, "cswap-tray — no data", "cswap-tray — sin datos")
    }
    pub fn no_accounts(self) -> &'static str {
        s!(
            self,
            "No managed accounts (run: cswap add)",
            "Sin cuentas gestionadas (usa: cswap add)"
        )
    }
    pub fn rotate(self) -> &'static str {
        s!(self, "Rotate to the next account", "Rotar a la siguiente cuenta")
    }
    pub fn refresh(self) -> &'static str {
        s!(self, "Refresh now", "Actualizar ahora")
    }
    pub fn dashboard(self) -> &'static str {
        s!(self, "Usage dashboard (cswap tui)", "Panel de uso (cswap tui)")
    }
    pub fn edit_appearance(self) -> &'static str {
        s!(self, "Edit appearance…", "Editar apariencia…")
    }
    pub fn quit(self) -> &'static str {
        s!(self, "Quit", "Salir")
    }
    pub fn account_fallback(self, n: u32) -> String {
        s!(self, format!("account {n}"), format!("cuenta {n}"))
    }

    // --- auto-switch submenu ----------------------------------------------
    pub fn auto_on(self, preferred: &str) -> String {
        s!(
            self,
            format!("Auto-switch: ON (returns to {preferred})"),
            format!("Cambio automático: ACTIVADO (vuelve a {preferred})")
        )
    }
    pub fn auto_off(self) -> &'static str {
        s!(self, "Auto-switch: off", "Cambio automático: desactivado")
    }
    pub fn enable(self) -> &'static str {
        s!(self, "Enable", "Activar")
    }
    pub fn disable(self) -> &'static str {
        s!(self, "Disable", "Desactivar")
    }
    pub fn not_set(self) -> &'static str {
        s!(self, "not set", "sin elegir")
    }
    pub fn preferred_account(self) -> &'static str {
        s!(self, "Preferred account:", "Cuenta preferida:")
    }
    pub fn auto_help(self, preferred: &str, switch_at: f32, back_below: f32, mins: u64) -> Vec<String> {
        s!(
            self,
            vec![
                format!("You work on {preferred}."),
                format!("When it reaches {switch_at:.0}% of its 5h or 7d quota,"),
                "it moves on its own to another account with room.".to_string(),
                format!("It returns to {preferred} once below {back_below:.0}%."),
                format!("Waits {mins} min between switches."),
                "If every account is spent, it does nothing.".to_string(),
                "Careful: if it fires mid-conversation,".to_string(),
                "that session continues on the other account.".to_string(),
            ],
            vec![
                format!("Trabajas en {preferred}."),
                format!("Si llega al {switch_at:.0}% de su cuota (5h o 7d),"),
                "pasa sola a otra cuenta con hueco.".to_string(),
                format!("Vuelve a {preferred} cuando baje del {back_below:.0}%."),
                format!("Espera {mins} min entre cambios."),
                "Si están todas agotadas, no hace nada.".to_string(),
                "Ojo: si salta a mitad de conversación,".to_string(),
                "esa sesión sigue en la otra cuenta.".to_string(),
            ]
        )
    }

    // --- prewarm submenu ---------------------------------------------------
    pub fn prewarm_title(self) -> &'static str {
        s!(self, "Prewarm the reserve", "Precalentar reserva")
    }
    pub fn prewarm_title_cold(self) -> &'static str {
        s!(
            self,
            "Prewarm the reserve ⚠ clock stopped",
            "Precalentar reserva ⚠ contador parado"
        )
    }
    pub fn prewarm_title_armed(self) -> &'static str {
        s!(
            self,
            "Prewarm: waiting for your message",
            "Precalentar: esperando tu mensaje"
        )
    }
    pub fn prewarm_armed_line(self, name: &str) -> String {
        s!(
            self,
            format!("Your next message goes through {name}."),
            format!("Tu próximo mensaje irá por {name}.")
        )
    }
    pub fn cancel_and_return(self) -> &'static str {
        s!(self, "Cancel and switch back", "Cancelar y volver")
    }
    pub fn already_running(self) -> &'static str {
        s!(self, " (already running)", " (ya en marcha)")
    }
    pub fn prewarm_arm(self, name: &str, suffix: &str) -> String {
        s!(
            self,
            format!("Send my next message through {name}{suffix}"),
            format!("Pasar mi próximo mensaje por {name}{suffix}")
        )
    }
    pub fn prewarm_task_run(self, name: &str) -> String {
        s!(
            self,
            format!("Run my prewarm task on {name}"),
            format!("Ejecutar mi tarea de precalentado en {name}")
        )
    }
    pub fn prewarm_task_unset(self) -> &'static str {
        s!(
            self,
            "No prewarm task set (see prewarm.task in the config)",
            "Sin tarea de precalentado (define prewarm.task en la config)"
        )
    }
    pub fn open_last_output(self) -> &'static str {
        s!(
            self,
            "Open the last prewarm answer",
            "Abrir la última respuesta de precalentado"
        )
    }
    pub fn task_started_title(self, name: &str) -> String {
        s!(
            self,
            format!("Running your task on {name}"),
            format!("Ejecutando tu tarea en {name}")
        )
    }
    pub fn task_started_body(self) -> &'static str {
        s!(
            self,
            "Its 5h window opens as a side effect. You will be told when the \
             answer is ready.",
            "Su ventana de 5 h se abre de paso. Te aviso cuando esté la \
             respuesta."
        )
    }
    pub fn task_done_title(self) -> &'static str {
        s!(self, "Prewarm task finished", "Tarea de precalentado terminada")
    }
    pub fn task_done_body(self) -> &'static str {
        s!(
            self,
            "The answer is in the menu, under 'Open the last prewarm answer'.",
            "La respuesta está en el menú, en 'Abrir la última respuesta de \
             precalentado'."
        )
    }
    pub fn prewarm_help(self) -> &'static [&'static str] {
        s!(
            self,
            &[
                "The 5h window opens with the first message,",
                "not with the login or the account switch.",
                "",
                "Prewarming does not reserve quota: it moves",
                "the clock forward, so the reserve renews",
                "sooner when you actually need it.",
                "",
                "'My next message': switches accounts and",
                "brings you back as soon as it lands.",
                "'My task': runs a prompt of your own on the",
                "reserve, without touching the account in use.",
                "Set it in prewarm.task — real work whose",
                "answer you read, not a hollow ping.",
            ][..],
            &[
                "La ventana de 5 h la abre el primer mensaje,",
                "no el login ni el cambio de cuenta.",
                "",
                "Precalentar no reserva cuota: adelanta el",
                "reloj, para que a la reserva se le renueve",
                "antes cuando la necesites de verdad.",
                "",
                "'Mi próximo mensaje': cambia de cuenta y te",
                "devuelve solo en cuanto detecta el mensaje.",
                "'Mi tarea': ejecuta un prompt tuyo en la",
                "reserva, sin tocar la cuenta que usas.",
                "Se define en prewarm.task: trabajo real cuya",
                "respuesta lees, no un ping hueco.",
            ][..]
        )
    }

    // --- icon legend -------------------------------------------------------
    pub fn legend_title(self) -> &'static str {
        s!(self, "What does the icon mean?", "¿Qué significa el icono?")
    }
    pub fn legend(self) -> &'static [&'static str] {
        s!(
            self,
            &[
                "Letter: the account in use right now.",
                "Left bar: usage of the last 5 hours.",
                "Right bar: usage of the last 7 days.",
                "They fill from the bottom up.",
                "Green <70%  ·  Amber 70-90%  ·  Red ≥90%",
                "Red pip in the corner: an account needs re-login.",
                "",
                "Left click: switch account.",
                "Right click: this menu.",
                "",
                "Accounts are added with 'cswap add'.",
            ][..],
            &[
                "Letra: cuenta activa ahora mismo.",
                "Barra izquierda: uso de las últimas 5 h.",
                "Barra derecha: uso de los últimos 7 días.",
                "Se llenan de abajo arriba.",
                "Verde <70%  ·  Ámbar 70-90%  ·  Rojo ≥90%",
                "Punto rojo en la esquina: una cuenta pide login.",
                "",
                "Clic izquierdo: cambiar de cuenta.",
                "Clic derecho: este menú.",
                "",
                "Las cuentas se añaden con 'cswap add'.",
            ][..]
        )
    }

    // --- re-login ----------------------------------------------------------
    /// Tag next to an account in the menu whose token cswap can no longer use.
    pub fn relogin_tag(self) -> &'static str {
        s!(self, "re-login needed", "requiere iniciar sesión")
    }
    pub fn relogin_title(self, name: &str) -> String {
        s!(
            self,
            format!("{name}: session expired"),
            format!("{name}: sesión caducada")
        )
    }
    pub fn relogin_body(self) -> &'static str {
        s!(
            self,
            "Open Claude Code on that account, run /login, then: cswap add",
            "Abre Claude Code en esa cuenta, usa /login y luego: cswap add"
        )
    }

    // --- notifications -----------------------------------------------------
    pub fn toast_test(self) -> &'static str {
        s!(self, "Notifications work.", "Los avisos funcionan.")
    }
    pub fn cold_title(self, name: &str) -> String {
        s!(
            self,
            format!("{name}'s clock is stopped"),
            format!("{name} tiene el contador parado")
        )
    }
    pub fn cold_body(self) -> &'static str {
        s!(
            self,
            "Its 5h window will not start until a message lands on it. \
             If you will need it today, prewarm it from the tray menu.",
            "Su ventana de 5 h no arrancará hasta que le entre un mensaje. \
             Si vas a necesitarla hoy, precaliéntala desde el menú de la bandeja."
        )
    }
    pub fn prewarming_title(self, name: &str) -> String {
        s!(self, format!("Prewarming {name}"), format!("Precalentando {name}"))
    }
    pub fn prewarming_body(self) -> &'static str {
        s!(
            self,
            "Your next message will go through that account, and you will be \
             switched back as soon as it is detected.",
            "Tu próximo mensaje irá por esa cuenta y te devuelvo a la tuya en \
             cuanto lo detecte."
        )
    }
    pub fn clock_started_title(self) -> &'static str {
        s!(self, "Clock started", "Contador arrancado")
    }
    pub fn clock_started_body(self, back: &str) -> String {
        s!(
            self,
            format!("The message landed on the reserve. You are back on {back}."),
            format!("El mensaje ha entrado en la reserva. Vuelves a {back}.")
        )
    }
    pub fn prewarm_cancelled_title(self) -> &'static str {
        s!(self, "Prewarm cancelled", "Precalentado cancelado")
    }
    pub fn prewarm_cancelled_body(self, back: &str) -> String {
        s!(
            self,
            format!("No message arrived in time. You are back on {back}."),
            format!("No detecté ningún mensaje a tiempo. Vuelves a {back}.")
        )
    }
    pub fn prewarm_failed(self) -> &'static str {
        s!(self, "Could not prewarm", "No se pudo precalentar")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spanish_langids_map_to_spanish() {
        // 0x0C0A es-ES, 0x2C0A es-AR, 0x080A es-MX.
        for id in [0x0C0Au16, 0x2C0A, 0x080A] {
            assert_eq!(Lang::from_langid(id), Lang::Es);
        }
    }

    #[test]
    fn everything_else_falls_back_to_english() {
        // 0x0409 en-US, 0x040C fr-FR, 0x0411 ja-JP.
        for id in [0x0409u16, 0x040C, 0x0411] {
            assert_eq!(Lang::from_langid(id), Lang::En);
        }
    }

    #[test]
    fn explicit_setting_wins_over_the_system() {
        assert_eq!(Lang::resolve("es"), Lang::Es);
        assert_eq!(Lang::resolve("EN"), Lang::En);
        assert_eq!(Lang::resolve(" es "), Lang::Es);
    }

    #[test]
    fn both_languages_cover_the_same_help_text() {
        assert_eq!(Lang::En.legend().len(), Lang::Es.legend().len());
        assert_eq!(Lang::En.prewarm_help().len(), Lang::Es.prewarm_help().len());
        assert_eq!(
            Lang::En.auto_help("X", 90.0, 80.0, 5).len(),
            Lang::Es.auto_help("X", 90.0, 80.0, 5).len()
        );
    }
}
