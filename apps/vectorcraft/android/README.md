# VectorCraft para Android & Huawei Tablets (M-Pencil / AOSP / GMS-Free)

Esta guía documenta la compilación, despliegue y arquitectura del soporte nativo de **VectorCraft en Android**, diseñado especialmente para tablets como la **Huawei MatePad 11.5** con el **Huawei M-Pencil (2ª Gen / NearLink)** y dispositivos AOSP sin Google Mobile Services (GMS).

---

## Características de la versión nativa

1. **100% Libre de GMS (Google Mobile Services)**:
   - No utiliza bibliotecas de Google Play Services, Firebase ni Google Play Billing.
   - Compatible nativamente con **HarmonyOS 3.1 / 4.0 / 4.2** y cualquier distribución AOSP / Android 12+.
2. **Soporte de Lápiz y Stylus de Precisión**:
   - **Sensibilidad a la presión**: Lee los 4.096 niveles de presión del Huawei M-Pencil a través de `AXIS_PRESSURE` y los normaliza en `0.0..=1.0` en el motor de trazo.
   - **Inclinación y Orientación**: Mapeo esférico de `AXIS_TILT` y `AXIS_ORIENTATION` hacia `PenPose` para brochas caligráficas y de dispersión.
   - **Muestreo de alta frecuencia (120Hz)**: Procesa lotes de muestras históricas (`get_history_size()`) para que los trazos rápidos no presenten esquinas ni cortes poligonales en la pantalla a 120Hz de la MatePad 11.5.
   - **Hover (Puntero flotante)**: Al aproximar el M-Pencil a menos de 1 cm de la pantalla (`ACTION_HOVER_MOVE`), el lienzo dibuja la mirilla/cursor de la herramienta activa con presión 0.
   - **Rechazo estricto de palma**: En cuanto se detecta el lápiz cerca o sobre el cristal, los contactos de los dedos o la palma apoyada quedan restringidos a navegación (zoom de pellizco, paneo de dos dedos o toques de 2 dedos para deshacer), evitando cualquier trazo involuntario.
   - **Botón del lápiz / Doble toque**: El botón lateral del stylus conmuta de inmediato a la herramienta Borrador (*Eraser*) o genera un clic secundario según la herramienta activa.

---

## Requisitos de compilación

1. **Rust Toolchain**:
   - `rustup target add aarch64-linux-android` (arquitectura de la MatePad 11.5)
   - Opcional para emulador de PC: `rustup target add x86_64-linux-android`
2. **Android NDK**:
   - NDK r25c o superior (`$ANDROID_NDK_HOME`).
3. **Herramienta de empaquetado**:
   - Se puede empaquetar con `cargo-apk` o `cargo-ndk`:
     ```bash
     cargo install cargo-apk
     # O alternativamente:
     cargo install cargo-ndk
     ```

---

## Compilación del APK para Huawei MatePad 11.5

### Con `cargo-apk`:
Desde la raíz del repositorio:
```bash
cargo apk build --release --target aarch64-linux-android --manifest-path apps/vectorcraft/Cargo.toml
```
El archivo APK generado estará ubicado en:
`target/release/apk/vectorcraft.apk`

### Con `cargo-ndk` + Gradle (si se integra en proyecto Android):
```bash
cargo ndk -t arm64-v8a -o android/app/src/main/jniLibs build --release -p vectorcraft
```

---

## Instalación en la MatePad 11.5

### Método 1: Por cable USB con ADB
1. Activa las **Opciones de desarrollador** y la **Depuración USB** en la tablet:
   - *Ajustes* ▸ *Acerca de la tableta* ▸ Toca 7 veces en *Número de compilación*.
   - *Ajustes* ▸ *Sistema y actualizaciones* ▸ *Opciones de desarrollador* ▸ Activar *Depuración por USB*.
2. Conecta la tableta a tu PC y ejecuta:
   ```bash
   adb install -r target/release/apk/vectorcraft.apk
   ```

### Método 2: Instalación directa (Sin PC)
1. Copia el archivo `vectorcraft.apk` a la memoria interna de la MatePad 11.5 (o mediante pendrive USB-C / Huawei Share).
2. Abre la app **Archivos** de Huawei, toca en `vectorcraft.apk` y selecciona **Instalar**.
3. Al no depender de GMS, la instalación se completa limpiamente y la app aparecerá en el menú de inicio lista para usar con el M-Pencil.
