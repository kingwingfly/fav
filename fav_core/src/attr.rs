pub trait Resource {
    fn id(&self) -> &str;
    fn title(&self) -> &str;
    fn upper(&self) -> &[impl Upper];
    fn belongs_to(&self) -> Option<&impl Resources>;
    fn belongs(&self, resources: impl Resources) -> bool {
        let id = self.id();
        resources.resources().iter().any(|r| r.id() == id)
    }
}

pub trait Resources {
    fn id(&self) -> &str;
    fn title(&self) -> &str;
    fn resources(&self) -> &[impl Resource];
    fn contains(&self, resource: impl Resource) -> bool {
        let id = resource.id();
        self.resources().iter().any(|r| r.id() == id)
    }
}

pub trait Upper {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Up;
    impl Upper for Up {
        fn id(&self) -> &str {
            "id"
        }
        fn name(&self) -> &str {
            "name"
        }
    }
    struct Res<'r> {
        uppers: Vec<Up>,
        belongs: Option<&'r Ress<'r>>,
    }
    impl Resource for Res<'_> {
        fn id(&self) -> &str {
            "id"
        }
        fn title(&self) -> &str {
            "title"
        }

        fn upper(&self) -> &[impl Upper] {
            &self.uppers
        }

        fn belongs_to(&self) -> Option<&impl Resources> {
            self.belongs
        }
    }
    struct Ress<'r> {
        resources: Vec<Res<'r>>,
    }
    impl Resources for Ress<'_> {
        fn id(&self) -> &str {
            "id"
        }
        fn title(&self) -> &str {
            "title"
        }
        fn resources(&self) -> &[impl Resource] {
            &self.resources
        }
    }
}
