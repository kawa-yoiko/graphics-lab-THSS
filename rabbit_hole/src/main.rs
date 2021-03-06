mod gl;
mod scene_loader;
mod ibl;
mod ray_tracer;

use glfw::Context;

use core::mem::{size_of, size_of_val};

fn check_gl_errors() {
  let err = gl::GetError();
  if err != 0 {
    panic!("OpenGL error: {}", err);
  }
}

const W: u32 = 960;
const H: u32 = 540;

fn shader(ty: gl::types::GLenum, src: &str) -> gl::types::GLuint {
  let id = gl::CreateShader(ty);
  gl::ShaderSource(
    id, 1,
    &(src.as_bytes().as_ptr().cast()),
    &(src.len() as gl::int),
  );
  gl::CompileShader(id);
  let mut log = [0u8; 1024];
  let mut len = 0;
  gl::GetShaderInfoLog(id, log.len() as gl::int,
    &mut len, log.as_mut_slice().as_mut_ptr().cast());
  if len != 0 {
    println!("{}", std::str::from_utf8(&log[..len as usize]).unwrap());
  }
  id
}

pub fn program(vs: &str, fs: &str) -> gl::types::GLuint {
  let vs = shader(gl::VERTEX_SHADER, vs);
  let fs = shader(gl::FRAGMENT_SHADER, fs);
  let prog = gl::CreateProgram();
  gl::AttachShader(prog, vs);
  gl::AttachShader(prog, fs);
  gl::LinkProgram(prog);
  gl::DeleteShader(vs);
  gl::DeleteShader(fs);
  prog
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let mut glfw = glfw::init(glfw::FAIL_ON_ERRORS).unwrap();

  glfw.window_hint(glfw::WindowHint::ContextVersion(3, 3));
  glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));
  glfw.window_hint(glfw::WindowHint::OpenGlProfile(glfw::OpenGlProfileHint::Core));

  let (mut window, events) = glfw.create_window(
    W, H,
    "Window",
    glfw::WindowMode::Windowed,
  )
    .expect("Cannot open window -- check graphics driver");

  gl::load_with(|s| window.get_proc_address(s) as *const _);
  glfw.set_swap_interval(glfw::SwapInterval::Sync(1));

  window.set_key_polling(true);
  window.make_current();

  // Load scene
  let frame = scene_loader::load("model/trees2.obj")?;

  // IBL
  let (start, end) = *frame.object_range.get("Metal_Icosphere")
    .ok_or("object Metal_Icosphere not found")?;
  let ibl = ibl::IBL::new(&frame.vertices[start..end]);

  // Sky box
  let skybox_tex = ibl.skybox();

  let skybox_prog = program(r"
    #version 330 core
    uniform mat4 VP;
    layout (location = 0) in vec3 v_pos;
    out vec3 f_tex_coord;

    void main() {
      f_tex_coord = v_pos;
      gl_Position = (VP * vec4(v_pos, 1.0)).xyww;
    }
  ", r"
    #version 330 core
    uniform samplerCube skybox;
    in vec3 f_tex_coord;
    out vec4 out_colour;

    void main() {
      vec3 s = texture(skybox, f_tex_coord).rgb;
      // Tone mapping
      s = s / (s + vec3(1));
      s = pow(s, vec3(1/2.2));
      out_colour = vec4(s, 1);
    }
  ");

  // Uniform locations
  let skybox_uni_vp = gl::GetUniformLocation(skybox_prog, "VP\0".as_ptr().cast());
  let skybox_uni_sampler = gl::GetUniformLocation(skybox_prog, "skybox\0".as_ptr().cast());
  gl::UseProgram(skybox_prog);
  gl::Uniform1i(skybox_uni_sampler, 0);

  let mut skybox_vao = 0;
  gl::GenVertexArrays(1, &mut skybox_vao);
  gl::BindVertexArray(skybox_vao);
  let mut skybox_vbo = 0;
  gl::GenBuffers(1, &mut skybox_vbo);
  gl::BindBuffer(gl::ARRAY_BUFFER, skybox_vbo);

  gl::EnableVertexAttribArray(0);
  gl::VertexAttribPointer(
    0,
    3, gl::FLOAT, gl::FALSE,
    (3 * size_of::<f32>()) as gl::int,
    0 as *const _,
  );

  let skybox_verts: [f32; 108] = include!("skybox_verts.txt");
  gl::BufferData(
    gl::ARRAY_BUFFER,
    size_of_val(&skybox_verts) as isize,
    skybox_verts.as_ptr().cast(),
    gl::STATIC_DRAW,
  );

  // Scene objects
  let scene_prog = program(r"
    #version 330 core
    uniform mat4 VP;
    uniform int mode; // 0 - opaque, 1 - semi-transparent
    layout (location = 0) in vec3 v_pos;
    layout (location = 1) in vec3 v_normal;
    layout (location = 2) in vec2 v_texcoord;
    layout (location = 3) in uint v_texid;
    out vec3 f_pos;
    out vec3 f_normal;
    out vec2 f_texcoord;
    out float f_texid;

    void main() {
      gl_Position = VP * vec4(v_pos, 1.0);
      if ((mode == 1) != (v_texcoord.x == -1))
        gl_Position.xyz = vec3(1e5);
      f_pos = v_pos;
      f_normal = v_normal;
      f_texcoord = v_texcoord;
      f_texid = v_texid;
    }
  ", r"
    #version 330 core
    uniform vec3 light_pos;
    uniform vec3 cam_pos;
    uniform sampler2D textures[16];
    in vec3 f_pos;
    in vec3 f_normal;
    in vec2 f_texcoord;
    in float f_texid;
    out vec4 out_colour;

    void main() {
      vec3 ambient_colour = vec3(0.1);
      vec3 light_colour = vec3(0.6);

      vec3 n = normalize(f_normal);

      vec3 light_dir = normalize(light_pos - f_pos);
      float diff = 0.7 * max(dot(n, light_dir), 0);

      vec3 view_dir = normalize(cam_pos - f_pos);
      vec3 h = normalize(view_dir + light_dir);
      float spec = 0.3 * pow(max(dot(n, h), 0.0), 16);

      out_colour = vec4(ambient_colour + (diff + spec) * light_colour, 1.0);
      if (f_texcoord.r < 0) { // Glass
        out_colour.rgb = vec3(1) - (vec3(1) - out_colour.rgb) * 0.5;
        out_colour.rgb *= vec3(0.45, 0.5, 0.6);
        out_colour.a = 0.4 + 0.4 * sqrt(1 - pow(dot(view_dir, n), 2));
      }
      if (f_texid < 16)
        out_colour *= texture(textures[int(f_texid + 0.5)], f_texcoord);
      else if (f_texcoord.r > 0)
        out_colour *= vec4(f_texcoord.rg, (f_texcoord.r + f_texcoord.g) * 0.3, 1);

      out_colour.rgb = pow(out_colour.rgb, vec3(1 / 2.2));
    }
  ");

  // Uniform locations
  let scene_uni_vp = gl::GetUniformLocation(scene_prog, "VP\0".as_ptr().cast());
  let scene_uni_mode = gl::GetUniformLocation(scene_prog, "mode\0".as_ptr().cast());
  let scene_uni_light_pos = gl::GetUniformLocation(scene_prog, "light_pos\0".as_ptr().cast());
  let scene_uni_cam_pos = gl::GetUniformLocation(scene_prog, "cam_pos\0".as_ptr().cast());
  // Textures
  gl::UseProgram(scene_prog);
  for i in 0..16 {
    let uni = gl::GetUniformLocation(scene_prog, format!("textures[{}]\0", i).as_ptr().cast());
    gl::Uniform1i(uni, i);
  }

  // Vertices
  let mut scene_vao = 0;
  gl::GenVertexArrays(1, &mut scene_vao);
  gl::BindVertexArray(scene_vao);
  let mut scene_vbo = 0;
  gl::GenBuffers(1, &mut scene_vbo);
  gl::BindBuffer(gl::ARRAY_BUFFER, scene_vbo);

  gl::EnableVertexAttribArray(0);
  gl::VertexAttribPointer(
    0,
    3, gl::FLOAT, gl::FALSE,
    size_of_val(&frame.vertices[0]) as gl::int,
    (0 * size_of::<f32>()) as *const _,
  );
  gl::EnableVertexAttribArray(1);
  gl::VertexAttribPointer(
    1,
    3, gl::FLOAT, gl::FALSE,
    size_of_val(&frame.vertices[0]) as gl::int,
    (3 * size_of::<f32>()) as *const _,
  );
  gl::EnableVertexAttribArray(2);
  gl::VertexAttribPointer(
    2,
    2, gl::FLOAT, gl::FALSE,
    size_of_val(&frame.vertices[0]) as gl::int,
    (6 * size_of::<f32>()) as *const _,
  );
  gl::EnableVertexAttribArray(3);
  gl::VertexAttribPointer(
    3,
    1, gl::UNSIGNED_BYTE, gl::FALSE,
    size_of_val(&frame.vertices[0]) as gl::int,
    (8 * size_of::<f32>()) as *const _,
  );

  // Load textures
  let mut scene_texs = vec![];
  for (w, h, buf) in &frame.textures {
    let mut mat_tex = 0;
    gl::GenTextures(1, &mut mat_tex);
    gl::BindTexture(gl::TEXTURE_2D, mat_tex);
    gl::TexImage2D(
      gl::TEXTURE_2D,
      0,
      gl::SRGB as gl::int,
      *w as gl::int, *h as gl::int, 0,
      gl::RGB,
      gl::UNSIGNED_BYTE,
      buf.as_ptr().cast()
    );
    gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as gl::int);
    gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as gl::int);
    gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::MIRRORED_REPEAT as gl::int);
    gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::MIRRORED_REPEAT as gl::int);
    scene_texs.push(mat_tex);
  }

  // Framebuffer
  let mut fbo = 0;
  gl::GenFramebuffers(1, &mut fbo);
  gl::BindFramebuffer(gl::FRAMEBUFFER, fbo);
  let (fb_w, fb_h) = window.get_framebuffer_size();
  // Colour texture
  let mut fb_tex_c = 0;
  gl::GenTextures(1, &mut fb_tex_c);
  gl::BindTexture(gl::TEXTURE_2D, fb_tex_c);
  gl::TexImage2D(
    gl::TEXTURE_2D,
    0,
    gl::RGB as gl::int,
    fb_w as gl::int, fb_h as gl::int, 0,
    gl::RGB,
    gl::UNSIGNED_BYTE,
    0 as *const _
  );
  gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as gl::int);
  gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as gl::int);
  gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::MIRRORED_REPEAT as gl::int);
  gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::MIRRORED_REPEAT as gl::int);
  gl::FramebufferTexture2D(gl::FRAMEBUFFER,
    gl::COLOR_ATTACHMENT0,
    gl::TEXTURE_2D, fb_tex_c, 0);
  // Depth texture
  let mut fb_tex_d = 0;
  gl::GenTextures(1, &mut fb_tex_d);
  gl::BindTexture(gl::TEXTURE_2D, fb_tex_d);
  gl::TexImage2D(
    gl::TEXTURE_2D,
    0,
    gl::DEPTH_COMPONENT32 as gl::int,
    fb_w as gl::int, fb_h as gl::int, 0,
    gl::DEPTH_COMPONENT,
    gl::UNSIGNED_INT,
    0 as *const _
  );
  gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as gl::int);
  gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as gl::int);
  gl::FramebufferTexture2D(gl::FRAMEBUFFER,
    gl::DEPTH_ATTACHMENT,
    gl::TEXTURE_2D, fb_tex_d, 0);
  // Unbind framebuffer
  gl::BindFramebuffer(gl::FRAMEBUFFER, 0);

  // Program for rendering to screen
  let fb_prog = program(r"
    #version 330 core
    layout (location = 0) in vec2 v_pos;
    out vec2 f_tex_coord;

    void main() {
      gl_Position = vec4(v_pos, 0.0, 1.0);
      f_tex_coord = vec2((1 + v_pos.x) / 2, (1 + v_pos.y) / 2);
    }
  ", r"
    #version 330 core
    uniform sampler2D tex;
    uniform sampler2D noise;
    in vec2 f_tex_coord;
    out vec4 out_colour;

    void main() {
      // Frosted glass
      vec2 offset = (texture(noise, f_tex_coord).rg - vec2(0.5, 0.5)) * 0.03;
      out_colour = vec4(0);
      for (int i = 0; i <= 4; i++) {
        out_colour += texture(tex, f_tex_coord + offset * i / 4) / 5;
      }
    }
  ");

  let fb_uni_tex = gl::GetUniformLocation(fb_prog, "tex\0".as_ptr().cast());
  let fb_uni_noise = gl::GetUniformLocation(fb_prog, "noise\0".as_ptr().cast());
  gl::UseProgram(fb_prog);
  gl::Uniform1i(fb_uni_tex, 0);
  gl::Uniform1i(fb_uni_noise, 1);

  // Noise texture
  let mut fb_tex_noise = 0;
  gl::GenTextures(1, &mut fb_tex_noise);
  gl::BindTexture(gl::TEXTURE_2D, fb_tex_noise);
  const NOISE_TEX_WIDTH: usize = 512;
  const NOISE_TEX_HEIGHT: usize = 288;
  let mut buf_noise = [[[0u8; 3]; NOISE_TEX_WIDTH]; NOISE_TEX_HEIGHT];
  let mut seed = 20211020u64;
  for i in 0..NOISE_TEX_HEIGHT {
    for j in 0..NOISE_TEX_WIDTH {
      for _ in 0..(i + j) % 17 {
        seed = (seed as u64 * 1103515245 + 12345) & 0x7fffffff;
      }
      seed = (seed as u64 * 1103515245 + 12345) & 0x7fffffff;
      buf_noise[i][j][0] = ((seed >> 16) & 0xff) as u8;
      seed = (seed as u64 * 1103515245 + 12345) & 0x7fffffff;
      buf_noise[i][j][1] = ((seed >> 16) & 0xff) as u8;
    }
  }
  gl::TexImage2D(
    gl::TEXTURE_2D,
    0,
    gl::RGB as gl::int,
    NOISE_TEX_WIDTH as gl::int, NOISE_TEX_HEIGHT as gl::int, 0,
    gl::RGB,
    gl::UNSIGNED_BYTE,
    buf_noise.as_ptr().cast()
  );
  gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as gl::int);
  gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as gl::int);

  // VAO and VBO
  let mut fb_vao = 0;
  gl::GenVertexArrays(1, &mut fb_vao);
  gl::BindVertexArray(fb_vao);
  let mut fb_vbo = 0;
  gl::GenBuffers(1, &mut fb_vbo);
  gl::BindBuffer(gl::ARRAY_BUFFER, fb_vbo);

  gl::EnableVertexAttribArray(0);
  gl::VertexAttribPointer(
    0,
    2, gl::FLOAT, gl::FALSE,
    (2 * size_of::<f32>()) as gl::int,
    0 as *const _,
  );

  let fb_verts: [f32; 12] = [
    -1.0, -1.0, -1.0,  1.0,  1.0,  1.0,
    -1.0, -1.0,  1.0, -1.0,  1.0,  1.0,
  ];
  gl::BufferData(
    gl::ARRAY_BUFFER,
    size_of_val(&fb_verts) as isize,
    fb_verts.as_ptr().cast(),
    gl::STATIC_DRAW,
  );

  // Extra texture and program for plain rendering
  let mut plain_tex = 0;
  gl::GenTextures(1, &mut plain_tex);
  gl::BindTexture(gl::TEXTURE_2D, plain_tex);
  gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as gl::int);
  gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as gl::int);

  let plain_prog = program(r"
    #version 330 core
    layout (location = 0) in vec2 v_pos;
    out vec2 f_tex_coord;

    void main() {
      gl_Position = vec4(v_pos, 0.0, 1.0);
      f_tex_coord = vec2((1 + v_pos.x) / 2, (1 + v_pos.y) / 2);
    }
  ", r"
    #version 330 core
    uniform sampler2D tex;
    in vec2 f_tex_coord;
    out vec4 out_colour;

    void main() {
      out_colour = texture(tex, f_tex_coord);
    }
  ");

  let plain_uni_tex = gl::GetUniformLocation(fb_prog, "tex\0".as_ptr().cast());
  gl::UseProgram(plain_prog);
  gl::Uniform1i(plain_uni_tex, 0);

  check_gl_errors();

  // Data kept between frames
  let mut last_time = glfw.get_time() as f32;

  let mut cam_pos = glm::vec3(0.0, 1.0, 10.0);
  let cam_up = glm::vec3(0.0, 1.0, 0.0);
  let mut cam_ori = glm::vec3(0.0, 0.0, -1.0);

  let p_mat = glm::ext::perspective(
    0.5236,
    16.0 / 9.0,
    0.1,
    100.0,
  );

  let mut scene_on = true;
  let mut last_scene_key_press = false;
  let mut metallic = 0.9;
  let mut roughness = 0.6;

  let mut raytrace_on = false;
  let mut last_raytrace_key_press = false;

  // let mut rt = ray_tracer::RayTracer::new(fb_w as u32, fb_h as u32, &frame);
  let debug_scale = 2i32;
  let mut rt = ray_tracer::RayTracer::new(fb_w as u32 / debug_scale as u32, fb_h as u32 / debug_scale as u32, &frame);
  gl::BindTexture(gl::TEXTURE_2D, plain_tex);
  gl::TexImage2D(
    gl::TEXTURE_2D,
    0,
    gl::RGBA as gl::int,
    fb_w as gl::int / debug_scale, fb_h as gl::int / debug_scale, 0,
    gl::RGBA,
    gl::UNSIGNED_BYTE,
    rt.image() as *const _
  );

  // Hide cursor
  window.set_cursor_mode(glfw::CursorMode::Disabled);
  window.set_sticky_keys(true);
  let mut last_cursor = window.get_cursor_pos();

  let keys_list = [
    glfw::Key::W, glfw::Key::Up,
    glfw::Key::S, glfw::Key::Down,
    glfw::Key::A, glfw::Key::Left,
    glfw::Key::D, glfw::Key::Right,
    glfw::Key::Q,
    glfw::Key::Z,
    glfw::Key::Num1,
    glfw::Key::Num2,
    glfw::Key::Num3,
    glfw::Key::Num4,
    glfw::Key::Num0,
  ];

  while !window.should_close() {
    window.swap_buffers();

    glfw.poll_events();
    for (_, event) in glfw::flush_messages(&events) {
      match event {
        glfw::WindowEvent::Key(glfw::Key::Escape, _, glfw::Action::Press, _) => {
          window.set_should_close(true)
        }
        _ => {}
      }
    }

    // Update with time
    let cur_time = glfw.get_time() as f32;
    let delta_time = cur_time - last_time;
    last_time = cur_time;

    // Frame data
    gl::BindVertexArray(scene_vao);
    gl::BindBuffer(gl::ARRAY_BUFFER, scene_vbo);
    gl::BufferData(
      gl::ARRAY_BUFFER,
      size_of_val(&*frame.vertices) as isize,
      frame.vertices.as_ptr().cast(),
      gl::STREAM_DRAW,
    );

    // Filter on?
    let filter_on =
      window.get_key(glfw::Key::LeftShift) == glfw::Action::Press;

    // Toggled ray tracing?
    let raytrace_key_press =
      window.get_key(glfw::Key::Space) == glfw::Action::Press;
    if !last_raytrace_key_press && raytrace_key_press {
      raytrace_on = !raytrace_on;
      if raytrace_on {
        rt.reset(cam_pos, cam_ori, cam_up, 0.5236, 10.0);
      } else {
        // Clear sticky key flags
        for k in keys_list { window.get_key(k); }
      }
    }
    last_raytrace_key_press = raytrace_key_press;

    // Navigation and patameters
    if !raytrace_on {
      // Camera navigation
      // Panning
      let move_dist = delta_time * 8.0;
      let cam_land = glm::normalize(cam_ori - glm::ext::projection(cam_ori, cam_up));
      if window.get_key(glfw::Key::W) == glfw::Action::Press
      || window.get_key(glfw::Key::Up) == glfw::Action::Press {
        cam_pos = cam_pos + cam_land * move_dist;
      }
      if window.get_key(glfw::Key::S) == glfw::Action::Press
      || window.get_key(glfw::Key::Down) == glfw::Action::Press {
        cam_pos = cam_pos - cam_land * move_dist;
      }
      if window.get_key(glfw::Key::A) == glfw::Action::Press
      || window.get_key(glfw::Key::Left) == glfw::Action::Press {
        cam_pos = cam_pos - glm::cross(cam_land, cam_up) * move_dist;
      }
      if window.get_key(glfw::Key::D) == glfw::Action::Press
      || window.get_key(glfw::Key::Right) == glfw::Action::Press {
        cam_pos = cam_pos + glm::cross(cam_land, cam_up) * move_dist;
      }
      if window.get_key(glfw::Key::Q) == glfw::Action::Press {
        cam_pos = cam_pos + cam_up * move_dist;
      }
      if window.get_key(glfw::Key::Z) == glfw::Action::Press {
        cam_pos = cam_pos - cam_up * move_dist;
      }

      // Rotation
      let (x, y) = window.get_cursor_pos();
      let (dx, dy) = (last_cursor.0 - x, last_cursor.1 - y);
      if dx.abs() >= 0.25 || dy.abs() >= 0.25 {
        let rotate_speed = 1.0 / 600.0;
        let cam_right = glm::cross(cam_ori, cam_up);
        // X
        let angle = dx as f32 * rotate_speed;
        let (cos_a, sin_a) = (angle.cos(), angle.sin());
        // cross(up, ori) = -right
        cam_ori = cam_ori * cos_a - cam_right * sin_a;
        // Y
        let angle = dy as f32 * rotate_speed;
        let orig_angle = glm::dot(cam_ori, cam_up).acos();
        let min_angle = 0.01;
        let angle = angle
          .min(-min_angle + orig_angle)
          .max(-std::f32::consts::PI + min_angle + orig_angle);
        let (cos_a, sin_a) = (angle.cos(), angle.sin());
        // cross(right, ori) = up
        cam_ori = cam_ori * cos_a + cam_up * sin_a;
        // In case drift happens
        cam_ori = glm::normalize(cam_ori);
      }

      // PBR parameters
      let adjustment = delta_time * 0.3;
      if window.get_key(glfw::Key::Num1) == glfw::Action::Press {
        metallic -= adjustment;
      }
      if window.get_key(glfw::Key::Num2) == glfw::Action::Press {
        metallic += adjustment;
      }
      metallic = metallic.max(0.0).min(1.0);
      if window.get_key(glfw::Key::Num3) == glfw::Action::Press {
        roughness -= adjustment;
      }
      if window.get_key(glfw::Key::Num4) == glfw::Action::Press {
        roughness += adjustment;
      }
      roughness = roughness.max(0.0).min(1.0);

      // Toggled scene on/off?
      let scene_key_press =
        window.get_key(glfw::Key::Num0) == glfw::Action::Press;
      if !last_scene_key_press && scene_key_press {
        scene_on = !scene_on;
      }
      last_scene_key_press = scene_key_press;
    }
    // Clear sticky key flags
    for k in keys_list { window.get_key(k); }

    if !raytrace_on || !rt.frame_filled() {
      // Camera matrix
      let v = glm::ext::look_at(cam_pos, cam_pos + cam_ori, cam_up);
      let vp = p_mat * v;
      let mut vnopan = v.clone();
      vnopan.c0.w = 0.0;
      vnopan.c1.w = 0.0;
      vnopan.c2.w = 0.0;
      vnopan.c3 = glm::vec4(0.0, 0.0, 0.0, 1.0);
      let vnopan_p = p_mat * vnopan;

      // Light
      let light_pos = glm::vec3(4.0, 1.0, 6.0);

      // Draw to framebuffer if filter is on
      if filter_on {
        gl::BindFramebuffer(gl::FRAMEBUFFER, fbo);
      } else {
        gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
      }

      // Draw
      // https://www.reddit.com/r/opengl/comments/1pxzzt/comment/cd79lxt/?utm_source=share&utm_medium=web2x&context=3
      gl::DepthMask(gl::TRUE);
      gl::ClearColor(1.0, 0.99, 0.99, 1.0);
      gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);

      gl::Disable(gl::BLEND);
      gl::Enable(gl::TEXTURE_CUBE_MAP_SEAMLESS);

      // Skybox
      gl::UseProgram(skybox_prog);
      // Uniforms and textures
      gl::UniformMatrix4fv(skybox_uni_vp, 1, gl::FALSE, vnopan_p.as_array().as_ptr().cast());
      gl::ActiveTexture(gl::TEXTURE0);
      gl::BindTexture(gl::TEXTURE_CUBE_MAP, skybox_tex);
      // Draw
      gl::BindVertexArray(skybox_vao);
      gl::BindBuffer(gl::ARRAY_BUFFER, skybox_vbo);
      gl::DepthMask(gl::FALSE);
      gl::Disable(gl::CULL_FACE);
      gl::Disable(gl::DEPTH_TEST);
      gl::DrawArrays(gl::TRIANGLES, 0, 36);

      // Scene
      gl::DepthMask(gl::TRUE);
      gl::Enable(gl::CULL_FACE);
      gl::Enable(gl::DEPTH_TEST);
      if scene_on {
        gl::UseProgram(scene_prog);
        // Textures
        for (i, mat_tex) in scene_texs.iter().enumerate() {
          gl::ActiveTexture(gl::TEXTURE0 + (i as u32));
          gl::BindTexture(gl::TEXTURE_2D, *mat_tex);
        }
        // Uniforms
        gl::UniformMatrix4fv(scene_uni_vp, 1, gl::FALSE, vp.as_array().as_ptr().cast());
        gl::Uniform3f(scene_uni_cam_pos, cam_pos.x, cam_pos.y, cam_pos.z);
        gl::Uniform3f(scene_uni_light_pos, light_pos.x, light_pos.y, light_pos.z);
        // Draw
        gl::BindVertexArray(scene_vao);
        gl::BindBuffer(gl::ARRAY_BUFFER, scene_vbo);
        // (1) Opaque objects
        gl::Uniform1i(scene_uni_mode, 0);
        gl::DepthFunc(gl::LESS);
        gl::DrawArrays(gl::TRIANGLES, 0, frame.vertices.len() as gl::int);
        // (2) Semi-transparent objects
        gl::Uniform1i(scene_uni_mode, 1);
        // gl::Disable(gl::CULL_FACE);
        gl::Enable(gl::BLEND);
        gl::BlendEquation(gl::FUNC_ADD);
        gl::BlendFunc(gl::ONE, gl::ONE_MINUS_SRC_ALPHA);
        gl::DrawArrays(gl::TRIANGLES, 0, frame.vertices.len() as gl::int);
      }

      // gl::Enable(gl::CULL_FACE);
      gl::Disable(gl::BLEND);
      gl::DepthFunc(gl::LEQUAL);
      ibl.draw(vp, cam_pos, light_pos, metallic, roughness);

      if filter_on {
        gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
        gl::UseProgram(fb_prog);
        gl::ActiveTexture(gl::TEXTURE0);
        gl::BindTexture(gl::TEXTURE_2D, fb_tex_c);
        gl::ActiveTexture(gl::TEXTURE1);
        gl::BindTexture(gl::TEXTURE_2D, fb_tex_noise);
        gl::BindVertexArray(fb_vao);
        gl::BindBuffer(gl::ARRAY_BUFFER, fb_vbo);
        gl::DepthMask(gl::FALSE);
        gl::Disable(gl::CULL_FACE);
        gl::Disable(gl::DEPTH_TEST);
        gl::DrawArrays(gl::TRIANGLES, 0, 6);
      }
    }

    if raytrace_on {
      // Render with the ray tracer
      rt.render();

      // Upload to texture
      gl::ActiveTexture(gl::TEXTURE0);
      gl::BindTexture(gl::TEXTURE_2D, plain_tex);
      gl::TexSubImage2D(
        gl::TEXTURE_2D,
        0,
        0, 0,
        fb_w as gl::int / debug_scale, fb_h as gl::int / debug_scale,
        gl::RGBA,
        gl::UNSIGNED_BYTE,
        rt.image() as *const _
      );

      if !rt.frame_filled() {
        gl::Enable(gl::BLEND);
        gl::BlendEquation(gl::FUNC_ADD);
        gl::BlendFunc(gl::ONE, gl::ONE_MINUS_SRC_ALPHA);
      }

      gl::UseProgram(plain_prog);
      gl::BindVertexArray(fb_vao);
      gl::BindBuffer(gl::ARRAY_BUFFER, fb_vbo);
      gl::DepthMask(gl::FALSE);
      gl::Disable(gl::CULL_FACE);
      gl::Disable(gl::DEPTH_TEST);
      gl::DrawArrays(gl::TRIANGLES, 0, 6);
    }

    last_cursor = window.get_cursor_pos();
    check_gl_errors();
  }

  Ok(())
}
