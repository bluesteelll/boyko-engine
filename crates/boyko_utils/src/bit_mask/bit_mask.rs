/* use std::ops::{BitAnd, BitOr, BitXor, Not, Sub};
use std::fmt::{Debug, Formatter, Result as FmtResult};
use anyhow::{Result, anyhow, bail, ensure};
use super::bit_set::BitSet;
use super::bit_storage::BitStorage;

/// A structure that stores a BitSet of active categories and a collection of BitSets 
/// for each category to track elements present in each category.
/// This allows for fast filtering of elements based on category presence.
#[derive(Clone)]
pub struct BitMask<T: BitStorage> {
    /// BitSet representing active categories
    categories: BitSet<T>,
    /// Array of BitSets, one for each category, tracking elements in each category
    elements: Vec<BitSet<T>>,
}

impl<T: BitStorage> BitMask<T> {
    /// Create a new BitMask with the given number of categories
    #[inline]
    pub fn new(category_count: usize) -> Self {
        let mut elements = Vec::with_capacity(category_count);
        elements.resize_with(category_count, BitSet::new);
        
        Self {
            categories: BitSet::new(),
            elements,
        }
    }
    
    /// Get the number of categories
    #[inline]
    pub fn category_count(&self) -> usize {
        self.elements.len()
    }
    
    /// Check if a category is active
    #[inline]
    pub fn is_category_active(&self, category: u32) -> bool {
        self.categories.is_set(category)
    }
    
    /// Set a category as active.
    /// Returns an error if the category index is out of bounds.
    #[inline]
    pub fn activate_category(&mut self, category: u32) -> Result<()> {
        let len = self.elements.len();
        ensure!(
            (category as usize) < len,
            "Category index out of bounds: index is {}, but length is {}",
            category, len
        );
        
        self.categories.set(category);
        Ok(())
    }
    
    /// Set a category as inactive.
    /// Returns an error if the category index is out of bounds.
    #[inline]
    pub fn deactivate_category(&mut self, category: u32) -> Result<()> {
        let len = self.elements.len();
        ensure!(
            (category as usize) < len,
            "Category index out of bounds: index is {}, but length is {}",
            category, len
        );
        
        self.categories.clear(category);
        Ok(())
    }
    
    /// Get the BitSet of active categories
    #[inline]
    pub fn active_categories(&self) -> &BitSet<T> {
        &self.categories
    }
    
    /// Set the BitSet of active categories
    #[inline]
    pub fn set_active_categories(&mut self, categories: BitSet<T>) -> Result<()> {
        // Ensure no bit is set for a category that doesn't exist
        for i in 0..T::from(8).count_ones() * std::mem::size_of::<T>() as u32 {
            if categories.is_set(i) && i as usize >= self.elements.len() {
                bail!("Cannot activate category {} as it is outside the range of existing categories (0-{})",
                     i, self.elements.len().saturating_sub(1));
            }
        }
        
        self.categories = categories;
        Ok(())
    }
    
    /// Check if an element is present in a specific category.
    /// Returns an error if the category index is out of bounds.
    #[inline]
    pub fn is_element_in_category(&self, element: u32, category: u32) -> Result<bool> {
        let cat_elements = self.get_category_elements(category)?;
        Ok(cat_elements.is_set(element))
    }
    
    /// Add an element to a specific category.
    /// Returns an error if the category index is out of bounds.
    #[inline]
    pub fn add_element_to_category(&mut self, element: u32, category: u32) -> Result<()> {
        let cat_elements = self.get_category_elements_mut(category)?;
        cat_elements.set(element);
        Ok(())
    }
    
    /// Remove an element from a specific category.
    /// Returns an error if the category index is out of bounds.
    #[inline]
    pub fn remove_element_from_category(&mut self, element: u32, category: u32) -> Result<()> {
        let cat_elements = self.get_category_elements_mut(category)?;
        cat_elements.clear(element);
        Ok(())
    }
    
    /// Get a reference to the BitSet for a specific category.
    /// Returns an error if the category index is out of bounds.
    #[inline]
    pub fn get_category_elements(&self, category: u32) -> Result<&BitSet<T>> {
        let index = category as usize;
        let len = self.elements.len();
        self.elements.get(index).ok_or_else(|| 
            anyhow!("Category index out of bounds: index is {}, but length is {}", 
                  category, len)
        )
    }
    
    /// Get a mutable reference to the BitSet for a specific category.
    /// Returns an error if the category index is out of bounds.
    #[inline]
    pub fn get_category_elements_mut(&mut self, category: u32) -> Result<&mut BitSet<T>> {
        let index = category as usize;
        let len = self.elements.len();
        self.elements.get_mut(index).ok_or_else(|| 
            anyhow!("Category index out of bounds: index is {}, but length is {}", 
                  category, len)
        )
    }
    
    /// Set the BitSet for a specific category.
    /// Returns an error if the category index is out of bounds.
    #[inline]
    pub fn set_category_elements(&mut self, category: u32, elements: BitSet<T>) -> Result<()> {
        let cat_elements = self.get_category_elements_mut(category)?;
        *cat_elements = elements;
        Ok(())
    }
    
    /// Get a BitSet representing elements that are present in all active categories
    pub fn elements_in_all_active_categories(&self) -> BitSet<T> {
        let mut active_categories_count = 0;
        let mut result: Option<BitSet<T>> = None;
        
        for (idx, elements) in self.elements.iter().enumerate() {
            let category = idx as u32;
            if !self.categories.is_set(category) {
                continue;
            }
            
            active_categories_count += 1;
            
            match result {
                Some(ref r) => result = Some(*r & *elements),
                None => result = Some(*elements),
            }
        }
        
        // If no active categories, return empty set
        if active_categories_count == 0 {
            return BitSet::new();
        }
        
        result.unwrap_or_else(BitSet::new)
    }
    
    /// Get a BitSet representing elements that are present in any active category
    pub fn elements_in_any_active_category(&self) -> BitSet<T> {
        let mut result = BitSet::new();
        
        for (idx, elements) in self.elements.iter().enumerate() {
            let category = idx as u32;
            if !self.categories.is_set(category) {
                continue;
            }
            
            result = result | *elements;
        }
        
        result
    }
    
    /// Filter another BitMask to only include elements that are in all active categories of this mask
    pub fn filter_all(&self, other: &BitMask<T>) -> BitMask<T> {
        // Get elements that are in all active categories of this mask
        let filtered_elements = self.elements_in_all_active_categories();
        
        // Create a new BitMask with the same number of categories as other
        let mut result = BitMask::new(other.category_count());
        
        // Copy active categories from other
        result.categories = other.categories;
        
        // For each category in other, filter its elements
        for (idx, elements) in other.elements.iter().enumerate() {
            let filtered_category_elements = *elements & filtered_elements;
            result.elements[idx] = filtered_category_elements;
        }
        
        result
    }
    
    /// Filter another BitMask to only include elements that are in any active category of this mask
    pub fn filter_any(&self, other: &BitMask<T>) -> BitMask<T> {
        // Get elements that are in any active category of this mask
        let filtered_elements = self.elements_in_any_active_category();
        
        // Create a new BitMask with the same number of categories as other
        let mut result = BitMask::new(other.category_count());
        
        // Copy active categories from other
        result.categories = other.categories;
        
        // For each category in other, filter its elements
        for (idx, elements) in other.elements.iter().enumerate() {
            let filtered_category_elements = *elements & filtered_elements;
            result.elements[idx] = filtered_category_elements;
        }
        
        result
    }
    
    /// Merge this BitMask with another using AND operation (intersection).
    /// If masks have different category counts, the result will have the maximum category count.
    pub fn intersection(&self, other: &BitMask<T>) -> BitMask<T> {
        let category_count = self.category_count().max(other.category_count());
        let mut result = BitMask::new(category_count);
        
        // Intersect active categories
        result.categories = self.categories & other.categories;
        
        // Intersect elements for each common category
        let min_category_count = self.category_count().min(other.category_count());
        for idx in 0..min_category_count {
            result.elements[idx] = self.elements[idx] & other.elements[idx];
        }
        
        result
    }
    
    /// Merge this BitMask with another using OR operation (union).
    /// If masks have different category counts, the result will have the maximum category count.
    pub fn union(&self, other: &BitMask<T>) -> BitMask<T> {
        let category_count = self.category_count().max(other.category_count());
        let mut result = BitMask::new(category_count);
        
        // Union active categories
        result.categories = self.categories | other.categories;
        
        // Process common categories
        let min_category_count = self.category_count().min(other.category_count());
        for idx in 0..min_category_count {
            result.elements[idx] = self.elements[idx] | other.elements[idx];
        }
        
        // Copy remaining categories from self
        if self.category_count() > min_category_count {
            for idx in min_category_count..self.category_count() {
                result.elements[idx] = self.elements[idx];
            }
        }
        
        // Copy remaining categories from other
        if other.category_count() > min_category_count {
            for idx in min_category_count..other.category_count() {
                result.elements[idx] = other.elements[idx];
            }
        }
        
        result
    }
    
    /// Calculate the difference between this BitMask and another (this - other).
    /// If masks have different category counts, the result will have the maximum category count.
    pub fn difference(&self, other: &BitMask<T>) -> BitMask<T> {
        let category_count = self.category_count().max(other.category_count());
        let mut result = BitMask::new(category_count);
        
        // Difference of active categories
        result.categories = self.categories - other.categories;
        
        // Process common categories
        let min_category_count = self.category_count().min(other.category_count());
        for idx in 0..min_category_count {
            result.elements[idx] = self.elements[idx] - other.elements[idx];
        }
        
        // Copy remaining categories from self
        if self.category_count() > min_category_count {
            for idx in min_category_count..self.category_count() {
                result.elements[idx] = self.elements[idx];
            }
        }
        
        result
    }
    
    /// Calculate the symmetric difference between this BitMask and another.
    /// If masks have different category counts, the result will have the maximum category count.
    pub fn symmetric_difference(&self, other: &BitMask<T>) -> BitMask<T> {
        let category_count = self.category_count().max(other.category_count());
        let mut result = BitMask::new(category_count);
        
        // Symmetric difference of active categories
        result.categories = self.categories ^ other.categories;
        
        // Process common categories
        let min_category_count = self.category_count().min(other.category_count());
        for idx in 0..min_category_count {
            result.elements[idx] = self.elements[idx] ^ other.elements[idx];
        }
        
        // Copy remaining categories from self
        if self.category_count() > min_category_count {
            for idx in min_category_count..self.category_count() {
                result.elements[idx] = self.elements[idx];
            }
        }
        
        // Copy remaining categories from other
        if other.category_count() > min_category_count {
            for idx in min_category_count..other.category_count() {
                result.elements[idx] = other.elements[idx];
            }
        }
        
        result
    }
    
    /// Create the complement of this BitMask
    pub fn complement(&self) -> BitMask<T> {
        let mut result = BitMask::new(self.category_count());
        
        // Complement of active categories
        result.categories = !self.categories;
        
        // Complement of elements for each category
        for idx in 0..self.category_count() {
            result.elements[idx] = !self.elements[idx];
        }
        
        result
    }
    
    /// Ensure that a category index is valid
    #[inline]
    fn ensure_valid_category(&self, category: u32) -> Result<()> {
        let len = self.elements.len();
        ensure!(
            (category as usize) < len,
            "Category index out of bounds: index is {}, but length is {}",
            category, len
        );
        Ok(())
    }
}

impl<T: BitStorage> Debug for BitMask<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("BitMask")
            .field("categories", &self.categories)
            .field("elements", &self.elements)
            .finish()
    }
}

impl<T: BitStorage> Default for BitMask<T> {
    #[inline]
    fn default() -> Self {
        Self::new(0)
    }
}

// Implement the bitwise operators for BitMask
impl<T: BitStorage> BitAnd for &BitMask<T> {
    type Output = BitMask<T>;

    #[inline]
    fn bitand(self, rhs: Self) -> Self::Output {
        self.intersection(rhs)
    }
}

impl<T: BitStorage> BitOr for &BitMask<T> {
    type Output = BitMask<T>;

    #[inline]
    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl<T: BitStorage> Sub for &BitMask<T> {
    type Output = BitMask<T>;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        self.difference(rhs)
    }
}

impl<T: BitStorage> BitXor for &BitMask<T> {
    type Output = BitMask<T>;

    #[inline]
    fn bitxor(self, rhs: Self) -> Self::Output {
        self.symmetric_difference(rhs)
    }
}

impl<T: BitStorage> Not for &BitMask<T> {
    type Output = BitMask<T>;

    #[inline]
    fn not(self) -> Self::Output {
        self.complement()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_category_activation() -> Result<()> {
        let mut mask: BitMask<u32> = BitMask::new(3);
        
        // Initial state - no active categories
        assert!(!mask.is_category_active(0));
        assert!(!mask.is_category_active(1));
        assert!(!mask.is_category_active(2));
        
        // Activate category 1
        mask.activate_category(1)?;
        assert!(!mask.is_category_active(0));
        assert!(mask.is_category_active(1));
        assert!(!mask.is_category_active(2));
        
        // Deactivate category 1
        mask.deactivate_category(1)?;
        assert!(!mask.is_category_active(0));
        assert!(!mask.is_category_active(1));
        assert!(!mask.is_category_active(2));
        
        // Attempt to activate an out-of-bounds category should fail
        assert!(mask.activate_category(3).is_err());
        
        Ok(())
    }
    
    #[test]
    fn test_elements_in_categories() -> Result<()> {
        let mut mask: BitMask<u32> = BitMask::new(3);
        
        // Add elements to categories
        mask.add_element_to_category(1, 0)?; // Element 1 in category 0
        mask.add_element_to_category(2, 0)?; // Element 2 in category 0
        mask.add_element_to_category(2, 1)?; // Element 2 in category 1
        mask.add_element_to_category(3, 1)?; // Element 3 in category 1
        mask.add_element_to_category(3, 2)?; // Element 3 in category 2
        mask.add_element_to_category(4, 2)?; // Element 4 in category 2
        
        // Check elements in categories
        assert!(mask.is_element_in_category(1, 0)?);
        assert!(mask.is_element_in_category(2, 0)?);
        assert!(!mask.is_element_in_category(3, 0)?);
        
        assert!(!mask.is_element_in_category(1, 1)?);
        assert!(mask.is_element_in_category(2, 1)?);
        assert!(mask.is_element_in_category(3, 1)?);
        
        assert!(!mask.is_element_in_category(2, 2)?);
        assert!(mask.is_element_in_category(3, 2)?);
        assert!(mask.is_element_in_category(4, 2)?);
        
        // Remove element from category
        mask.remove_element_from_category(2, 0)?;
        assert!(!mask.is_element_in_category(2, 0)?);
        
        // Out of bounds access should fail
        assert!(mask.is_element_in_category(1, 5).is_err());
        assert!(mask.add_element_to_category(1, 5).is_err());
        assert!(mask.remove_element_from_category(1, 5).is_err());
        
        Ok(())
    }
    
    #[test]
    fn test_elements_in_active_categories() -> Result<()> {
        let mut mask: BitMask<u32> = BitMask::new(3);
        
        // Add elements to categories
        mask.add_element_to_category(1, 0)?; // Element 1 in category 0
        mask.add_element_to_category(2, 0)?; // Element 2 in category 0
        mask.add_element_to_category(2, 1)?; // Element 2 in category 1
        mask.add_element_to_category(3, 1)?; // Element 3 in category 1
        mask.add_element_to_category(3, 2)?; // Element 3 in category 2
        mask.add_element_to_category(4, 2)?; // Element 4 in category 2
        
        // Activate categories 0 and 1
        mask.activate_category(0)?;
        mask.activate_category(1)?;
        
        // Elements in all active categories should be element 2
        let all_active = mask.elements_in_all_active_categories();
        assert!(!all_active.is_set(1));
        assert!(all_active.is_set(2));
        assert!(!all_active.is_set(3));
        assert!(!all_active.is_set(4));
        
        // Elements in any active category should be elements 1, 2, 3
        let any_active = mask.elements_in_any_active_category();
        assert!(any_active.is_set(1));
        assert!(any_active.is_set(2));
        assert!(any_active.is_set(3));
        assert!(!any_active.is_set(4));
        
        Ok(())
    }
    
    #[test]
    fn test_bitwise_operations() -> Result<()> {
        let mut mask1: BitMask<u32> = BitMask::new(2);
        let mut mask2: BitMask<u32> = BitMask::new(2);
        
        // Setup mask1: Category 0 active with elements 1, 2
        mask1.activate_category(0)?;
        mask1.add_element_to_category(1, 0)?;
        mask1.add_element_to_category(2, 0)?;
        
        // Setup mask2: Category 1 active with elements 2, 3
        mask2.activate_category(1)?;
        mask2.add_element_to_category(2, 1)?;
        mask2.add_element_to_category(3, 1)?;
        
        // Test intersection (AND)
        let intersection = &mask1 & &mask2;
        assert!(!intersection.is_category_active(0));
        assert!(!intersection.is_category_active(1));
        
        // Test union (OR)
        let union = &mask1 | &mask2;
        assert!(union.is_category_active(0));
        assert!(union.is_category_active(1));
        assert!(union.is_element_in_category(1, 0)?);
        assert!(union.is_element_in_category(2, 0)?);
        assert!(union.is_element_in_category(2, 1)?);
        assert!(union.is_element_in_category(3, 1)?);
        
        // Test difference (SUB)
        let difference = &mask1 - &mask2;
        assert!(difference.is_category_active(0));
        assert!(!difference.is_category_active(1));
        
        // Test symmetric difference (XOR)
        let sym_diff = &mask1 ^ &mask2;
        assert!(sym_diff.is_category_active(0));
        assert!(sym_diff.is_category_active(1));
        
        // Test complement (NOT)
        let complement = !&mask1;
        assert!(!complement.is_category_active(0));
        assert!(complement.is_category_active(1));
        
        Ok(())
    }
    
    #[test]
    fn test_set_active_categories() -> Result<()> {
        let mut mask: BitMask<u32> = BitMask::new(2);
        
        // Valid set
        let mut categories = BitSet::new();
        categories.set(0);
        mask.set_active_categories(categories)?;
        assert!(mask.is_category_active(0));
        assert!(!mask.is_category_active(1));
        
        // Invalid set - category 2 doesn't exist
        let mut invalid_categories = BitSet::new();
        invalid_categories.set(2);
        assert!(mask.set_active_categories(invalid_categories).is_err());
        
        Ok(())
    }
} */ 