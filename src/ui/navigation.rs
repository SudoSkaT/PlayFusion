//! Navegación de listas compartida (spec §14): las vistas con lista (Search
//! selecciona resultados; Related/Now Playing seleccionan recomendaciones)
//! exponen su iteración a través de [`ListSelection`] y la UI maneja `W`/`S`
//! y `↑`/`↓` con una única lógica de selección con wrap.

/// Índice resultante de mover la selección dentro de una lista de `len`
/// elementos (con wrap). Sin selección previa cae al primero; lista vacía
/// devuelve `None`. Función pura: la mutación de la selección es del dueño.
pub fn step_selection(len: usize, selected: Option<usize>, forward: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    match selected {
        None => Some(0),
        Some(i) => {
            let next = if forward {
                (i + 1) % len
            } else {
                (i + len - 1) % len
            };
            Some(next)
        }
    }
}

/// Contrato de navegación de una lista seleccionable.
pub trait ListSelection {
    /// Total de elementos de la lista.
    fn list_len(&self) -> usize;

    /// Índice seleccionado actual (`None` = nada).
    fn cursor(&self) -> Option<usize>;

    /// Fija la selección (`None` la limpia).
    fn set_cursor(&mut self, index: Option<usize>);

    /// Avanza (`forward = true`) o retrocede la selección con wrap.
    /// Devuelve el índice resultante (`None` si la lista está vacía).
    fn step(&mut self, forward: bool) -> Option<usize> {
        let next = step_selection(self.list_len(), self.cursor(), forward);
        self.set_cursor(next);
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeList {
        len: usize,
        sel: Option<usize>,
    }

    impl ListSelection for FakeList {
        fn list_len(&self) -> usize {
            self.len
        }
        fn cursor(&self) -> Option<usize> {
            self.sel
        }
        fn set_cursor(&mut self, index: Option<usize>) {
            self.sel = index;
        }
    }

    #[test]
    fn step_selection_no_selection_falls_to_first() {
        assert_eq!(step_selection(3, None, true), Some(0));
        assert_eq!(step_selection(3, None, false), Some(0));
    }

    #[test]
    fn step_selection_empty_list_stays_none() {
        assert_eq!(step_selection(0, None, true), None);
        assert_eq!(step_selection(0, Some(3), false), None);
    }

    #[test]
    fn step_selection_wraps_both_ways() {
        assert_eq!(step_selection(3, Some(2), true), Some(0), "forward wrap");
        assert_eq!(step_selection(3, Some(0), false), Some(2), "backward wrap");
        assert_eq!(step_selection(1, Some(0), true), Some(0));
    }

    #[test]
    fn trait_step_mutates_the_cursor() {
        let mut list = FakeList { len: 3, sel: None };
        assert_eq!(list.step(true), Some(0));
        assert_eq!(list.step(true), Some(1));
        assert_eq!(list.step(false), Some(0));
        assert_eq!(list.step(false), Some(2));
    }
}
