pub struct ParticipantInfo {
   layout: Layout,
   count: usize
}


pub trait Participant {
    pub fn new();


    pub fn to_bytes();
    pub fn from_bytes<T: usize>() -> [Entity; T];


}